use anyhow::{Result, ensure};
use tracing::instrument;

use super::valid_octal_mode;

const MAX_SECRET_FILE_NAME_LEN: usize = 255;

/// Check a secret file's name and mode. The name is a single file name, used as a path part on
/// the server and in the container.
#[instrument(level = "debug")]
pub fn validate_secret_file(name: &str, mode: u32) -> Result<()> {
    ensure!(
        valid_name(name),
        "Secret file name '{name}' must be a file name of up to {MAX_SECRET_FILE_NAME_LEN} \
         letters, digits and . _ - + @, other than . and .."
    );
    ensure!(
        valid_octal_mode(mode),
        "Secret file mode {mode} for '{name}' must be octal digits up to 777"
    );
    Ok(())
}

fn valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_SECRET_FILE_NAME_LEN || name == "." || name == ".." {
        return false;
    }
    for byte in name.bytes() {
        if !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'@')) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_single_safe_file_names() {
        for name in ["token", "db.password", "api_key-2", "user+1@host"] {
            validate_secret_file(name, 444).unwrap();
        }
        let long = "a".repeat(256);
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "../x",
            "a b",
            "a\nb",
            "$(id)",
            long.as_str(),
        ] {
            assert!(validate_secret_file(name, 444).is_err(), "{name:?}");
        }
    }

    #[test]
    fn modes_are_octal_digits_up_to_777() {
        for mode in [0, 400, 440, 444, 600, 777] {
            validate_secret_file("token", mode).unwrap();
        }
        for mode in [8, 448, 800, 1000, 7777] {
            assert!(validate_secret_file("token", mode).is_err(), "{mode}");
        }
    }
}
