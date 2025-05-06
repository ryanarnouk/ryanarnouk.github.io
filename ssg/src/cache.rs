use log::info;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Error, Read};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FileCache {
    pub file_data: HashMap<PathBuf, FileMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileMetadata {
    pub modified: SystemTime,
    pub hash: Option<String>,
}

// Struct representing a specific cache that is initialized for the given JSON file (as specified
// as the path buffer)
pub struct CacheContext {
    pub path: PathBuf,
    pub cache: FileCache,
}

impl CacheContext {
    pub fn load_or_default(path: PathBuf) -> Result<Self, Error> {
        let cache = if path.exists() {
            load_cache(&path)
        } else {
            FileCache::default()
        };
        Ok(Self { path, cache })
    }

    // Based on the metadata modified date and the hash, return true if the file has changed. False
    // otherwise
    pub fn has_file_changed(&self, path: &PathBuf) -> Result<bool, Error> {
        let current_metadata = compute_file_metadata(path)?;
        if let Some(cached_metadata) = self.cache.file_data.get(path) {
            if cached_metadata.modified == current_metadata.modified
                || cached_metadata.hash == current_metadata.hash
            {
                return Ok(false); // If not modified, and hash is the same file has not changed
            } else {
                return Ok(true);
            }
        }

        Ok(true)
    }

    // Checks if the file has changed (and updates the cache accordingly after updating)
    // Or does not perform the update (leaving the same results)
    // Returns true if the file was changed or false if otherwise
    pub fn update_file_if_changed(&mut self, file_path: &PathBuf) -> io::Result<bool> {
        let metadata = compute_file_metadata(file_path)?;
        if self.has_file_changed(file_path)? {
            self.cache
                .file_data
                .insert(file_path.to_path_buf(), metadata);
            // Save the updated cache
            println!(
                "File path {:?} with cache path as {:?}",
                file_path, self.path
            );

            let _ = save_cache(&self.cache, &self.path);
            Ok(true)
        } else {
            info!("Skipping unchanged file: {:?}", file_path);
            Ok(false)
        }
    }
}

// Save the cache to the disk
pub fn save_cache(cache: &FileCache, path: &PathBuf) -> Result<(), Error> {
    let json = serde_json::to_string(cache)?;
    fs::write(path, json)
}

// Load the cache from the disk
pub fn load_cache(path: &PathBuf) -> FileCache {
    let data = fs::read_to_string(path).unwrap();
    serde_json::from_str(&data).unwrap()
}

// Compute the file metadata (uses blake3 hashing algorithm)
pub fn compute_file_metadata(path: &PathBuf) -> Result<FileMetadata, Error> {
    let metadata = fs::metadata(path)?;
    let modified = metadata.modified()?; // last mod time in metadata

    // compute hash of file content
    let mut file = File::open(path)?;
    let mut content = Vec::new();
    let _ = file.read_to_end(&mut content);
    let hash = Some(blake3::hash(&content).to_hex().to_string());

    Ok(FileMetadata { modified, hash })
}
