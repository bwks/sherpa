use std::env;
use std::fs;
use std::path::Path;

/// Get server URL from environment variable
///
/// Checks for SHERPA_SERVER_URL environment variable
pub fn get_server_url() -> Option<String> {
    env::var("SHERPA_SERVER_URL").ok()
}

/// Read a value from a KEY=VALUE env file.
///
/// Lines starting with `#` and empty lines are skipped.
/// Returns `None` if the file doesn't exist or the key isn't found.
pub fn read_env_file_value(path: &Path, key: &str) -> Option<String> {
    tracing::debug!(path = %path.display(), key, "Reading env file");
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Failed to read env file");
            return None;
        }
    };
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=')
            && k.trim() == key
        {
            tracing::debug!(key, "Found key in env file");
            return Some(v.trim().to_string());
        }
    }
    tracing::debug!(key, path = %path.display(), "Key not found in env file");
    None
}
