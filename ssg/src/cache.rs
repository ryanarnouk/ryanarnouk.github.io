// Incremental build cache.
//
// The cache maps every file the SSG writes (keyed by its path relative to the output directory)
// to a fingerprint of everything that went into producing it. An output is only skipped when its
// fingerprint is unchanged AND the file still exists on disk. Because the fingerprint covers all
// inputs (source file, templates, site-wide page listing, SSG binary, ...) a change anywhere that
// affects an output invalidates it, rather than only changes to the output's own source file.
//
// Entries that are not produced again during a build belong to deleted/renamed sources. Their
// output files are removed from the output directory and dropped from the cache.

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

// Bump whenever the cache format or fingerprint scheme changes so old caches are discarded
const CACHE_VERSION: u32 = 2;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    // output path (relative to the output directory, '/' separated) -> fingerprint
    entries: HashMap<String, String>,
}

pub struct BuildCache {
    path: PathBuf,
    output_dir: PathBuf,
    // When false, nothing is skipped (but the cache is still written for the next build)
    enabled: bool,
    previous: HashMap<String, String>,
    current: HashMap<String, String>,
}

impl BuildCache {
    pub fn load(path: PathBuf, output_dir: PathBuf, enabled: bool) -> Self {
        let previous = match fs::read_to_string(&path) {
            Ok(data) => match serde_json::from_str::<CacheFile>(&data) {
                Ok(file) if file.version == CACHE_VERSION => file.entries,
                Ok(_) => {
                    info!("Cache {:?} is from an older SSG version, discarding", path);
                    HashMap::new()
                }
                Err(e) => {
                    warn!("Cache {:?} is corrupt ({}), discarding", path, e);
                    HashMap::new()
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                warn!("Could not read cache {:?} ({}), discarding", path, e);
                HashMap::new()
            }
        };

        Self {
            path,
            output_dir,
            enabled,
            previous,
            current: HashMap::new(),
        }
    }

    // Returns true if `output` (relative to the output directory) is up to date for the given
    // fingerprint. Either way, the output is marked as produced by this build.
    pub fn is_fresh(&mut self, output: &str, fingerprint: &str) -> bool {
        self.current
            .insert(output.to_string(), fingerprint.to_string());

        self.enabled
            && self.previous.get(output).map(String::as_str) == Some(fingerprint)
            && self.output_dir.join(output).is_file()
    }

    // Mark an output that failed to build so that it is retried on the next run (without treating
    // it as stale and deleting the previous version of it)
    pub fn invalidate(&mut self, output: &str) {
        self.current.insert(output.to_string(), String::new());
    }

    // Delete outputs from previous builds whose source no longer exists, then persist the cache.
    // The cache is written to a temporary file and renamed so an interrupted write cannot leave
    // behind a truncated cache.
    pub fn finish(self) -> io::Result<()> {
        for stale in self
            .previous
            .keys()
            .filter(|k| !self.current.contains_key(*k))
        {
            if !is_safe_relative_path(stale) {
                warn!("Ignoring suspicious cache entry {:?}", stale);
                continue;
            }
            let stale_path = self.output_dir.join(stale);
            match fs::remove_file(&stale_path) {
                Ok(()) => info!("Removed stale output {:?}", stale_path),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => warn!("Could not remove stale output {:?}: {}", stale_path, e),
            }
        }

        let file = CacheFile {
            version: CACHE_VERSION,
            entries: self.current,
        };
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = self.path.with_extension("json.tmp");
        fs::write(&tmp_path, serde_json::to_vec_pretty(&file)?)?;
        fs::rename(&tmp_path, &self.path)?;
        debug!("Saved cache to {:?}", self.path);
        Ok(())
    }
}

// Only allow plain relative paths so a tampered cache can never delete files outside the output
// directory
fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

// Incrementally builds a blake3 fingerprint. Every part is length-prefixed so that different
// sequences of parts can never produce the same byte stream.
#[derive(Clone)]
pub struct Fingerprint(blake3::Hasher);

impl Fingerprint {
    pub fn new() -> Self {
        Self(blake3::Hasher::new())
    }

    pub fn add(&mut self, bytes: impl AsRef<[u8]>) -> &mut Self {
        let bytes = bytes.as_ref();
        self.0.update(&(bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
        self
    }

    pub fn finish(&self) -> String {
        self.0.finalize().to_hex().to_string()
    }
}

// Converts a path relative to the output directory into a stable cache key
pub fn output_key(relative: &Path) -> String {
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    fn load(dir: &TempDir, enabled: bool) -> BuildCache {
        BuildCache::load(
            dir.path().join("cache.json"),
            dir.path().join("out"),
            enabled,
        )
    }

    #[test]
    fn rejects_paths_outside_output_dir() {
        assert!(is_safe_relative_path("index.html"));
        assert!(is_safe_relative_path("static/images/a.webp"));
        assert!(!is_safe_relative_path("../a.html"));
        assert!(!is_safe_relative_path("static/../../a.html"));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path("./a.html"));
        assert!(!is_safe_relative_path(""));
    }

    #[test]
    fn fingerprint_is_deterministic_and_unambiguous() {
        let a = Fingerprint::new().add("ab").add("c").finish();
        assert_eq!(a, Fingerprint::new().add("ab").add("c").finish());
        assert_ne!(a, Fingerprint::new().add("a").add("bc").finish());
        assert_ne!(a, Fingerprint::new().add("abc").finish());
    }

    #[test]
    fn output_key_uses_forward_slashes() {
        let path: PathBuf = ["static", "images", "a.webp"].iter().collect();
        assert_eq!(output_key(&path), "static/images/a.webp");
    }

    #[test]
    fn first_build_is_never_fresh() {
        let dir = TempDir::new();
        dir.write("out/a.html", "a");
        assert!(!load(&dir, true).is_fresh("a.html", "1"));
    }

    #[test]
    fn fresh_when_fingerprint_matches_and_output_exists() {
        let dir = TempDir::new();
        dir.write("out/a.html", "a");
        let mut cache = load(&dir, true);
        cache.is_fresh("a.html", "1");
        cache.finish().unwrap();

        let mut cache = load(&dir, true);
        assert!(cache.is_fresh("a.html", "1"));
    }

    #[test]
    fn stale_when_fingerprint_changes() {
        let dir = TempDir::new();
        dir.write("out/a.html", "a");
        let mut cache = load(&dir, true);
        cache.is_fresh("a.html", "1");
        cache.finish().unwrap();

        assert!(!load(&dir, true).is_fresh("a.html", "2"));
    }

    #[test]
    fn stale_when_output_was_deleted() {
        let dir = TempDir::new();
        let output = dir.write("out/a.html", "a");
        let mut cache = load(&dir, true);
        cache.is_fresh("a.html", "1");
        cache.finish().unwrap();

        fs::remove_file(output).unwrap();
        assert!(!load(&dir, true).is_fresh("a.html", "1"));
    }

    #[test]
    fn disabled_cache_never_skips_but_is_still_saved() {
        let dir = TempDir::new();
        dir.write("out/a.html", "a");
        let mut cache = load(&dir, false);
        assert!(!cache.is_fresh("a.html", "1"));
        cache.finish().unwrap();

        assert!(load(&dir, true).is_fresh("a.html", "1"));
    }

    #[test]
    fn corrupt_or_outdated_cache_is_discarded() {
        let dir = TempDir::new();
        dir.write("out/a.html", "a");

        dir.write("cache.json", "{not json");
        assert!(!load(&dir, true).is_fresh("a.html", "1"));

        dir.write(
            "cache.json",
            r#"{"version": 1, "entries": {"a.html": "1"}}"#,
        );
        assert!(!load(&dir, true).is_fresh("a.html", "1"));

        dir.write(
            "cache.json",
            format!(r#"{{"version": {CACHE_VERSION}, "entries": {{"a.html": "1"}}}}"#),
        );
        assert!(load(&dir, true).is_fresh("a.html", "1"));
    }

    #[test]
    fn outputs_not_produced_again_are_removed() {
        let dir = TempDir::new();
        let kept = dir.write("out/kept.html", "a");
        let removed = dir.write("out/static/removed.webp", "b");
        let mut cache = load(&dir, true);
        cache.is_fresh("kept.html", "1");
        cache.is_fresh("static/removed.webp", "1");
        cache.finish().unwrap();

        let mut cache = load(&dir, true);
        cache.is_fresh("kept.html", "1");
        cache.finish().unwrap();

        assert!(kept.exists());
        assert!(!removed.exists());
    }

    #[test]
    fn unsafe_entries_never_delete_files() {
        let dir = TempDir::new();
        let outside = dir.write("outside.html", "keep me");
        dir.write(
            "cache.json",
            format!(r#"{{"version": {CACHE_VERSION}, "entries": {{"../outside.html": "1"}}}}"#),
        );
        load(&dir, true).finish().unwrap();
        assert!(outside.exists());
    }

    #[test]
    fn invalidated_outputs_are_retried_but_not_deleted() {
        let dir = TempDir::new();
        let output = dir.write("out/a.html", "a");
        let mut cache = load(&dir, true);
        cache.is_fresh("a.html", "1");
        cache.finish().unwrap();

        let mut cache = load(&dir, true);
        cache.is_fresh("a.html", "1");
        cache.invalidate("a.html");
        cache.finish().unwrap();

        assert!(output.exists());
        assert!(!load(&dir, true).is_fresh("a.html", "1"));
    }

    #[test]
    fn cache_is_written_atomically() {
        let dir = TempDir::new();
        load(&dir, true).finish().unwrap();
        assert!(dir.path().join("cache.json").exists());
        assert!(!dir.path().join("cache.json.tmp").exists());
    }
}
