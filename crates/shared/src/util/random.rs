use anyhow::{Result, anyhow};
use twox_hash::XxHash32;

use super::user::get_username;

/// Generate a random two-word lab name (e.g. "adorable-badger").
pub fn generate_lab_name() -> Result<String> {
    petname::petname(2, "-").ok_or_else(|| anyhow!("Failed to generate lab name"))
}

/// Get the xxHash of a username and lab name.
/// The hash will be a maximum of 8 chars long.
pub fn get_id(lab_name: &str) -> Result<String> {
    let username = get_username()?;
    Ok(get_id_for_user(&username, lab_name))
}

/// Get the xxHash of an explicit username and lab name.
/// The hash will be a maximum of 8 chars long.
pub fn get_id_for_user(username: &str, lab_name: &str) -> String {
    let combined = format!("{}{}", username, lab_name);
    let seed = 4_294_967_295;
    let hash = XxHash32::oneshot(seed, combined.as_bytes());
    format!("{:08x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // generate_lab_name
    // ========================================================================

    #[test]
    fn test_generate_lab_name_returns_ok() {
        let name = generate_lab_name().expect("should generate a name");
        assert!(!name.is_empty());
    }

    #[test]
    fn test_generate_lab_name_contains_hyphen() {
        let name = generate_lab_name().expect("should generate a name");
        assert!(
            name.contains('-'),
            "expected two words joined by hyphen, got: {name}"
        );
    }

    #[test]
    fn test_generate_lab_name_has_two_words() {
        let name = generate_lab_name().expect("should generate a name");
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 2, "expected exactly two words, got: {name}");
    }

    #[test]
    fn test_generate_lab_name_only_lowercase_alpha_and_hyphens() {
        let name = generate_lab_name().expect("should generate a name");
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "expected only lowercase ascii and hyphens, got: {name}"
        );
    }

    // ========================================================================
    // get_id
    // ========================================================================

    #[test]
    fn test_get_id_returns_8_char_hex() {
        let id = get_id("test-lab").expect("generates id");
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_get_id_deterministic() {
        let id1 = get_id("my-lab").expect("generates id");
        let id2 = get_id("my-lab").expect("generates id");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_get_id_different_names_different_ids() {
        let id1 = get_id("lab-alpha").expect("generates id");
        let id2 = get_id("lab-beta").expect("generates id");
        assert_ne!(id1, id2);
    }

    // ========================================================================
    // get_id_for_user
    // ========================================================================

    #[test]
    fn test_get_id_for_user_returns_8_char_hex() {
        let id = get_id_for_user("alice", "test-lab");
        assert_eq!(id.len(), 8);
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "expected hex string, got: {id}"
        );
    }

    #[test]
    fn test_get_id_for_user_deterministic() {
        let id1 = get_id_for_user("bob", "my-lab");
        let id2 = get_id_for_user("bob", "my-lab");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_get_id_for_user_different_usernames_different_ids() {
        let id1 = get_id_for_user("alice", "same-lab");
        let id2 = get_id_for_user("bob", "same-lab");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_get_id_for_user_different_lab_names_different_ids() {
        let id1 = get_id_for_user("alice", "lab-one");
        let id2 = get_id_for_user("alice", "lab-two");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_get_id_for_user_empty_inputs() {
        let id = get_id_for_user("", "");
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
