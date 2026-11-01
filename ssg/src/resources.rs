// Handles the use of static resources within the website (stored
// in the static folder). This also handles image optimization
// workflows: raster images are downscaled (if needed) and converted to WebP. Every other file
// (favicons, SVGs, GIFs, fonts, PDFs, ...) is copied over as-is.

use crate::cache::{output_key, BuildCache, Fingerprint};
use image::ImageFormat;
use log::{debug, info};
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// Bump whenever the image pipeline changes so cached optimized images are regenerated
const PIPELINE_VERSION: &str = "static-v2";

const MAX_WIDTH: u32 = 1920;
const MAX_HEIGHT: u32 = 1080;

// Extensions (lowercase) of images that are converted to WebP
const CONVERTED_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "bmp", "tiff"];

fn is_converted(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| CONVERTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

// The file name a static resource is written to within the output static folder. Shared with the
// markdown parser so image links always point at the file that was actually produced.
pub fn output_file_name(relative: &str) -> String {
    let path = Path::new(relative);
    if is_converted(path) {
        output_key(&path.with_extension("webp"))
    } else {
        relative.to_string()
    }
}

fn optimize_image(source: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let image = image::open(source)?;

    // Only ever shrink images (preserving the aspect ratio), never upscale small ones
    let image = if image.width() > MAX_WIDTH || image.height() > MAX_HEIGHT {
        image.resize(MAX_WIDTH, MAX_HEIGHT, image::imageops::FilterType::Lanczos3)
    } else {
        image
    };

    // Encode into memory first so a failed encode never leaves a partial file behind
    let mut encoded = std::io::Cursor::new(Vec::new());
    image.write_to(&mut encoded, ImageFormat::WebP)?;
    fs::write(output, encoded.into_inner())?;
    Ok(())
}

// Optimize & copy the static folder into `output_dir`/`static_output_dir_name`
pub fn optimize_and_copy_static_folder(
    static_dir: &Path,
    output_dir: &Path,
    static_output_dir_name: &str,
    cache: &mut BuildCache,
) -> Result<(), Box<dyn Error>> {
    if !static_dir.is_dir() {
        return Err(format!("Static folder {:?} does not exist", static_dir).into());
    }

    // Detect two sources mapping to the same output (e.g. photo.png and photo.jpg -> photo.webp)
    let mut produced_by: HashMap<String, PathBuf> = HashMap::new();

    let entries = WalkDir::new(static_dir)
        .sort_by_file_name()
        .into_iter()
        // Skip hidden files and folders (.DS_Store, ...)
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));

    for entry in entries {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let source = entry.path();
        let relative = output_key(source.strip_prefix(static_dir)?);
        let output_relative = format!("{}/{}", static_output_dir_name, output_file_name(&relative));

        if let Some(other) = produced_by.insert(output_relative.clone(), source.to_path_buf()) {
            return Err(format!(
                "Static files {:?} and {:?} would both be written to {:?}",
                other, source, output_relative
            )
            .into());
        }

        let contents = fs::read(source)?;
        let fingerprint = Fingerprint::new()
            .add(PIPELINE_VERSION)
            .add(&output_relative)
            .add(&contents)
            .finish();

        if cache.is_fresh(&output_relative, &fingerprint) {
            debug!("Skipping unchanged static file: {:?}", source);
            continue;
        }

        let output = output_dir.join(&output_relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }

        let result = if is_converted(source) {
            info!("Optimizing static image: {:?}", source);
            optimize_image(source, &output)
                .map_err(|e| format!("Could not optimize image {:?}: {}", source, e).into())
        } else {
            info!("Copying static file: {:?}", source);
            fs::write(&output, &contents).map_err(Into::into)
        };

        if let Err(e) = result {
            cache.invalidate(&output_relative);
            return Err(e);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    fn write_png(dir: &TempDir, relative: &str, width: u32, height: u32) {
        let path = dir.write(relative, "");
        image::RgbImage::new(width, height).save(path).unwrap();
    }

    fn build(dir: &TempDir, enabled: bool) -> Result<(), Box<dyn Error>> {
        let mut cache = BuildCache::load(
            dir.path().join("cache.json"),
            dir.path().join("out"),
            enabled,
        );
        optimize_and_copy_static_folder(
            &dir.path().join("static"),
            &dir.path().join("out"),
            "static",
            &mut cache,
        )?;
        cache.finish()?;
        Ok(())
    }

    fn output_dimensions(dir: &TempDir, relative: &str) -> (u32, u32) {
        let image = image::open(dir.path().join("out/static").join(relative)).unwrap();
        (image.width(), image.height())
    }

    #[test]
    fn output_file_names() {
        assert_eq!(output_file_name("images/a.png"), "images/a.webp");
        assert_eq!(output_file_name("images/a.JPG"), "images/a.webp");
        assert_eq!(
            output_file_name("images/my photo.jpeg"),
            "images/my photo.webp"
        );
        assert_eq!(output_file_name("a.svg"), "a.svg");
        assert_eq!(output_file_name("a.gif"), "a.gif");
        assert_eq!(output_file_name("favicon.ico"), "favicon.ico");
        assert_eq!(output_file_name("docs/resume.pdf"), "docs/resume.pdf");
        assert_eq!(output_file_name("README"), "README");
    }

    #[test]
    fn converts_images_to_webp() {
        let dir = TempDir::new();
        write_png(&dir, "static/images/a.png", 40, 20);
        build(&dir, true).unwrap();

        let output = dir.path().join("out/static/images/a.webp");
        assert_eq!(
            image::ImageFormat::from_path(&output).unwrap(),
            ImageFormat::WebP
        );
        assert_eq!(output_dimensions(&dir, "images/a.webp"), (40, 20));
        assert!(!dir.path().join("out/static/images/a.png").exists());
    }

    #[test]
    fn downscales_large_images_keeping_aspect_ratio() {
        let dir = TempDir::new();
        write_png(&dir, "static/wide.png", 3840, 1080);
        write_png(&dir, "static/tall.png", 1000, 2160);
        build(&dir, true).unwrap();

        assert_eq!(output_dimensions(&dir, "wide.webp"), (1920, 540));
        assert_eq!(output_dimensions(&dir, "tall.webp"), (500, 1080));
    }

    #[test]
    fn copies_other_files_verbatim() {
        let dir = TempDir::new();
        dir.write("static/icons/logo.svg", "<svg></svg>");
        dir.write("static/favicon.ico", [0u8, 0, 1, 0, 9, 9]);
        build(&dir, true).unwrap();

        let out = dir.path().join("out/static");
        assert_eq!(
            fs::read_to_string(out.join("icons/logo.svg")).unwrap(),
            "<svg></svg>"
        );
        assert_eq!(
            fs::read(out.join("favicon.ico")).unwrap(),
            [0u8, 0, 1, 0, 9, 9]
        );
    }

    #[test]
    fn skips_hidden_files_and_folders() {
        let dir = TempDir::new();
        dir.write("static/.DS_Store", "junk");
        dir.write("static/.hidden/a.txt", "junk");
        dir.write("static/a.txt", "a");
        build(&dir, true).unwrap();

        let out = dir.path().join("out/static");
        assert!(out.join("a.txt").exists());
        assert!(!out.join(".DS_Store").exists());
        assert!(!out.join(".hidden").exists());
    }

    #[test]
    fn rejects_files_that_map_to_the_same_output() {
        let dir = TempDir::new();
        write_png(&dir, "static/a.png", 4, 4);
        dir.write("static/a.webp", "already webp");
        let error = build(&dir, true).unwrap_err().to_string();
        assert!(error.contains("would both be written to"), "{error}");
    }

    #[test]
    fn invalid_images_fail_the_build() {
        let dir = TempDir::new();
        dir.write("static/broken.png", "not a png");
        let error = build(&dir, true).unwrap_err().to_string();
        assert!(error.contains("Could not optimize image"), "{error}");
        assert!(!dir.path().join("out/static/broken.webp").exists());
    }

    #[test]
    fn missing_static_folder_is_an_error() {
        let dir = TempDir::new();
        assert!(build(&dir, true).is_err());
    }

    #[test]
    fn unchanged_files_are_not_rewritten() {
        let dir = TempDir::new();
        dir.write("static/a.txt", "a");
        build(&dir, true).unwrap();

        // Change the output behind the cache's back: an unchanged source must leave it alone
        let output = dir.write("out/static/a.txt", "edited");
        build(&dir, true).unwrap();
        assert_eq!(fs::read_to_string(&output).unwrap(), "edited");

        // A changed source is copied again
        dir.write("static/a.txt", "b");
        build(&dir, true).unwrap();
        assert_eq!(fs::read_to_string(&output).unwrap(), "b");

        // With the cache disabled everything is copied again
        dir.write("out/static/a.txt", "edited");
        build(&dir, false).unwrap();
        assert_eq!(fs::read_to_string(&output).unwrap(), "b");
    }

    #[test]
    fn deleted_sources_are_removed_from_the_output() {
        let dir = TempDir::new();
        let source = dir.write("static/a.txt", "a");
        build(&dir, true).unwrap();
        assert!(dir.path().join("out/static/a.txt").exists());

        fs::remove_file(source).unwrap();
        build(&dir, true).unwrap();
        assert!(!dir.path().join("out/static/a.txt").exists());
    }
}
