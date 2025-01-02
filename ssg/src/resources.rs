// Handles the use of static resources within the website (stored
// in the static folder). This also handles image optimization
// workflows

use crate::cache::{self, compute_file_metadata};
use image::{DynamicImage, ImageFormat};
use log::info;
use std::fs;
use std::io::Result;
use std::path::{Path, PathBuf};

fn save_optimized_image(image: &DynamicImage, output_path: &Path) -> std::io::Result<()> {
    // resize image to a max width (e.g., 1920px)
    let resized = image.resize(1920, 1080, image::imageops::FilterType::Lanczos3);

    // generate a new file name with a WebP extension
    let output_path = output_path.with_extension("webp");

    // write the resized image as WebP
    let mut output_file = std::fs::File::create(output_path)?;
    resized
        .write_to(&mut output_file, ImageFormat::WebP)
        .unwrap();

    Ok(())
}

// Optimize & copy static folder
pub fn optimize_and_copy_static_folder(
    static_path: &Path,
    static_output_path: &Path,
    cache_path: &PathBuf,
) -> Result<()> {
    let mut cache = cache::load_cache(&cache_path).unwrap_or_default();

    if !static_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Static folder does not exist",
        ));
    }

    for entry in fs::read_dir(static_path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            let file_name = path.file_name().unwrap();
            let static_output_path = static_output_path.join(file_name);

            let file_name_path_buf = path.to_path_buf();
            // check if the file is unmodified (already in build)
            if cache::has_file_changed(&file_name_path_buf, &cache)? {
                info!("Copying/optimizing static file: {:?}", file_name_path_buf);
            } else {
                info!("Skipping already copied file: {:?}", file_name_path_buf);
                continue;
            }

            // Proceed with optimization and copying
            let image = image::io::Reader::open(&path)?.decode().unwrap();
            save_optimized_image(&image, &static_output_path)?;

            // Update the cache (after the optimization occurs)
            cache.file_data.insert(
                file_name_path_buf.clone(),
                compute_file_metadata(&file_name_path_buf)?,
            );

            let _ = cache::save_cache(&cache, &cache_path);
        } else if path.is_dir() {
            // Recursively process subdirectories
            let subfolder = static_output_path.join(path.file_name().unwrap());
            fs::create_dir_all(&subfolder)?;
            optimize_and_copy_static_folder(&path, &subfolder, cache_path)?;
        }
    }

    Ok(())
}
