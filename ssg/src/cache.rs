use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Error};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FileCache {
    pub file_data: HashMap<PathBuf, FileMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileMetadata {
    pub modified: SystemTime,
    pub hash: Option<String>,
}

// Save the cache to the disk
pub fn save_cache(cache: &FileCache, path: &PathBuf) -> Result<(), Error> {
    let json = serde_json::to_string(cache)?;
    fs::write(path, json)
}

// Load the cache from the disk
pub fn load_cache(path: &PathBuf) -> Result<FileCache, Error> {
    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
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

pub fn has_file_changed(path: &PathBuf, cache: &FileCache) -> Result<bool, Error> {
    let current_metadata = compute_file_metadata(path)?;
    if let Some(cached_metadata) = cache.file_data.get(path) {
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
