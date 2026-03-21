use anyhow::Result;
use twox_hash::XxHash32;

use super::user::get_username;

/// Get the xxHash of a username and current working directory
/// The hash will be a maximum of 8 chars long.
pub fn get_id(lab_name: &str) -> Result<String> {
    let combined = format!("{}{}", get_username()?, lab_name);
    let seed = 4_294_967_295;
    let hash = XxHash32::oneshot(seed, combined.as_bytes());
    Ok(format!("{:08x}", hash))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
