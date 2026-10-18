use anyhow::{Result, ensure};
use tracing::instrument;

use crate::konst::{
    RESERVED_USERNAME_PREFIXES, RESERVED_USERNAMES, USERNAME_MAX_LENGTH, USERNAME_MIN_LENGTH,
};

/// Validate a Sherpa server username.
///
/// Server usernames double as guest login names on lab nodes, so they follow the guest rules:
/// 3–32 lowercase ASCII letters, digits, underscores or hyphens, starting with a letter or
/// underscore, and not a reserved name. Callers normalise the input with
/// `util::normalize_username` before validating it.
#[instrument(level = "debug")]
pub fn validate_username(username: &str) -> Result<()> {
    ensure!(
        username.len() >= USERNAME_MIN_LENGTH && is_portable(username),
        "Username must be {USERNAME_MIN_LENGTH}–{USERNAME_MAX_LENGTH} lowercase letters, digits, underscores or hyphens, starting with a letter or underscore"
    );
    ensure!(
        !is_reserved(username, RESERVED_USERNAMES),
        "Username '{username}' is reserved"
    );
    Ok(())
}

/// Validate a portable Linux username and conflicts with generated guest accounts.
#[instrument(level = "debug")]
pub fn validate_guest_username(username: &str, reserved: &[&str]) -> Result<()> {
    ensure!(
        !username.is_empty() && is_portable(username),
        "Guest username must be 1–{USERNAME_MAX_LENGTH} lowercase ASCII letters, digits, underscores or hyphens, starting with a letter or underscore"
    );
    ensure!(
        username != "root" && !is_reserved(username, reserved),
        "Guest username conflicts with a generated or privileged account: {username}"
    );
    Ok(())
}

fn is_portable(username: &str) -> bool {
    let bytes = username.as_bytes();
    bytes.len() <= USERNAME_MAX_LENGTH
        && bytes
            .first()
            .is_some_and(|c| c.is_ascii_lowercase() || *c == b'_')
        && bytes
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

fn is_reserved(username: &str, reserved: &[&str]) -> bool {
    reserved.contains(&username)
        || RESERVED_USERNAME_PREFIXES
            .iter()
            .any(|prefix| username.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konst::{ADMIN_USERNAME, SHERPA_USERNAME};

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
        for name in ["root", "sherpa", "core", "systemd-network"] {
            assert!(validate_guest_username(name, &["sherpa", "core"]).is_err());
        }
    }

    #[test]
    fn server_usernames_follow_the_guest_rules() {
        for name in [
            "bradmin",
            "alice",
            "_svc",
            "user-12",
            "a_b",
            "abc",
            &"a".repeat(32),
        ] {
            assert!(validate_username(name).is_ok(), "{name:?}");
        }
        for name in [
            "ab",
            "",
            "Brad",
            "1brad",
            "-brad",
            "brad.searle",
            "brad@example.com",
            "josé",
            "brad smith",
            &"a".repeat(33),
        ] {
            let error = validate_username(name).unwrap_err().to_string();
            assert!(error.contains("lowercase letters"), "{name:?}: {error}");
        }
    }

    #[test]
    fn server_usernames_reject_every_reserved_name() {
        for name in RESERVED_USERNAMES
            .iter()
            .copied()
            .filter(|name| name.len() >= USERNAME_MIN_LENGTH)
        {
            let error = validate_username(name).unwrap_err().to_string();
            assert!(error.contains("reserved"), "{name:?}: {error}");
        }
        for name in [ADMIN_USERNAME, SHERPA_USERNAME, "root", "systemd-resolve"] {
            assert!(validate_username(name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn reserved_names_are_valid_portable_names() {
        // A reserved name that broke the character rules could never be chosen anyway.
        for name in RESERVED_USERNAMES {
            assert!(is_portable(name), "{name:?}");
        }
    }
}
