use std::env;
use std::path::PathBuf;

// Resolve a base path either from an environment variable (for build/release versions)
// or a default relative path (during debugging with cargo run)
pub fn resolve_base_path(env_var_name: &str, fallback_relative_path: &str) -> PathBuf {
    if let Ok(env_var_path) = env::var(env_var_name) {
        PathBuf::from(env_var_path)
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(fallback_relative_path)
    }
}
