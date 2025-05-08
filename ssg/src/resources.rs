// Handles the use of static resources within the website (stored
// in the static folder). This also handles image optimization
// workflows

use crate::cache::CacheContext;
use image::{DynamicImage, ImageFormat};
use log::{info, warn};
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
    let cache_file = cache_path.clone();
    let mut cache_context = CacheContext::load_or_default(cache_file).unwrap();

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
            // if not, skip this file (continue), otherwise, update the cache
            // and log reflecting that a new resource will save the optimized copy
            if !cache_context.update_file_if_changed(&file_name_path_buf)? {
                info!("Skipping already copied file: {:?}", file_name_path_buf);
                continue;
            } else {
                info!("Copying/optimizing static file: {:?}", file_name_path_buf);
            }

            let reader = image::io::Reader::open(&path)?;
            let format = reader.format().unwrap_or(ImageFormat::Png); // fallback if format is not detected
            match reader.decode() {
                Ok(image) => {
                    if format == ImageFormat::Ico {
                        // generate a new file name with a WebP extension
                        let output_path = static_output_path.with_extension("ico");

                        // write the resized image as WebP
                        let mut output_file = std::fs::File::create(output_path)?;
                        image.write_to(&mut output_file, ImageFormat::Ico).unwrap();
                    } else {
                        save_optimized_image(&image, &static_output_path)?;
                    }
                }
                Err(_err) => {
                    warn!("Could not decode the following file as an image (optimization failed): {:?}", path);
                }
            }
        } else if path.is_dir() {
            // Recursively process subdirectories
            let subfolder = static_output_path.join(path.file_name().unwrap());
            fs::create_dir_all(&subfolder)?;
            optimize_and_copy_static_folder(&path, &subfolder, &cache_path)?;
        }
    }

    Ok(())
}
