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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_env(content: &str) -> tempfile::TempPath {
        let mut tmp = tempfile::NamedTempFile::new().expect("creates temp file");
        tmp.write_all(content.as_bytes()).expect("writes content");
        tmp.into_temp_path()
    }

    #[test]
    fn test_read_env_file_value_found() {
        let path = write_temp_env("DB_HOST=localhost\nDB_PORT=5432\n");
        assert_eq!(
            read_env_file_value(&path, "DB_PORT"),
            Some("5432".to_string())
        );
    }

    #[test]
    fn test_read_env_file_value_not_found() {
        let path = write_temp_env("DB_HOST=localhost\n");
        assert_eq!(read_env_file_value(&path, "MISSING_KEY"), None);
    }

    #[test]
    fn test_read_env_file_value_skips_comments() {
        let path = write_temp_env("# DB_PORT=9999\nDB_PORT=5432\n");
        assert_eq!(
            read_env_file_value(&path, "DB_PORT"),
            Some("5432".to_string())
        );
    }

    #[test]
    fn test_read_env_file_value_skips_blank_lines() {
        let path = write_temp_env("\n\nKEY=value\n\n");
        assert_eq!(read_env_file_value(&path, "KEY"), Some("value".to_string()));
    }

    #[test]
    fn test_read_env_file_value_missing_file() {
        let path = Path::new("/tmp/nonexistent_sherpa_env_file_test");
        assert_eq!(read_env_file_value(path, "KEY"), None);
    }

    #[test]
    fn test_read_env_file_value_trims_whitespace() {
        let path = write_temp_env("  KEY  =  value  \n");
        assert_eq!(read_env_file_value(&path, "KEY"), Some("value".to_string()));
    }
}
