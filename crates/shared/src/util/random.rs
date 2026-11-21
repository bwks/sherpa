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

    #[test]
    fn lab_identity_004_a_new_lab_id_is_a_stable_hash_of_the_owner_and_the_lab_name() {
        assert_eq!(get_id_for_user("bob", "my-lab"), "05f076b9");
        assert_ne!(
            get_id_for_user("alice", "same-lab"),
            get_id_for_user("bob", "same-lab")
        );
        assert_ne!(
            get_id_for_user("alice", "lab-one"),
            get_id_for_user("alice", "lab-two")
        );
    }
}
