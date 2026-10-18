use anyhow::{Result, ensure};
use tracing::instrument;

/// Validate a portable Linux username and conflicts with generated guest accounts.
#[instrument(level = "debug")]
pub fn validate_guest_username(username: &str, reserved: &[&str]) -> Result<()> {
    let bytes = username.as_bytes();
    ensure!(
        !bytes.is_empty()
            && bytes.len() <= 32
            && (bytes[0].is_ascii_lowercase() || bytes[0] == b'_')
            && bytes
                .iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-'),
        "Guest username must be 1–32 lowercase ASCII letters, digits, underscores or hyphens, starting with a letter or underscore"
    );
    ensure!(
        username != "root" && !reserved.contains(&username),
        "Guest username conflicts with a generated or privileged account: {username}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_names_and_length_boundaries() {
        for name in ["alice", "_a", "user-12", "a_b", &"a".repeat(32)] {
            assert!(validate_guest_username(name, &["sherpa"]).is_ok());
        }
        for name in [
            "",
            "Alice",
            "1user",
            "-user",
            "a.b",
            "a@b",
            "é",
            "alice\n",
            &"a".repeat(33),
        ] {
            assert!(validate_guest_username(name, &[]).is_err(), "{name:?}");
        }
    }

    #[test]
    fn rejects_privileged_and_generated_accounts() {
        for name in ["root", "sherpa", "core"] {
            assert!(validate_guest_username(name, &["sherpa", "core"]).is_err());
        }
    }
}
