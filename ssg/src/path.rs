use std::env;
use std::path::PathBuf;

// Resolve a path either from an environment variable (for build/release versions set within
// the GitHub Actions runner)
// or resolves a relative path to an absolute path (suitable for use when debugging with cargo run)
// NOTE: When using the default behaviour, the fallback relative path may be relative to the file
// path observed when using cargo run. The built executable may be relative to a different directory
pub fn resolve_environment_variable_path(
    env_var_name: &str,
    fallback_relative_path: &str,
) -> PathBuf {
    if let Ok(env_var_path) = env::var(env_var_name) {
        PathBuf::from(env_var_path)
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(fallback_relative_path)
    }
}
