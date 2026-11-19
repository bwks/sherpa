use anyhow::{Result, anyhow};
use twox_hash::XxHash32;

/// Generate a random two-word lab name (e.g. "adorable-badger").
///
/// # Errors
///
/// Fails when no name could be generated.
pub fn generate_lab_name() -> Result<String> {
    petname::petname(2, "-").ok_or_else(|| anyhow!("Failed to generate lab name"))
}

/// A lab's ID: the xxHash of its owner's Sherpa username and its name, 8 hex characters.
#[must_use]
pub fn get_id_for_user(username: &str, lab_name: &str) -> String {
    let combined = format!("{username}{lab_name}");
    let seed = 4_294_967_295;
    let hash = XxHash32::oneshot(seed, combined.as_bytes());
    format!("{hash:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // generate_lab_name
    // ========================================================================

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
    fn test_get_id_for_user_is_a_stable_hash_of_user_and_lab() {
        assert_eq!(get_id_for_user("bob", "my-lab"), "05f076b9");
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
}
