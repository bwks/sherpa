/// Normalise a Sherpa server username for storage and lookup.
///
/// Usernames are case-insensitive: they are trimmed and stored in lower case, and every lookup
/// lowercases its input the same way.
#[must_use]
pub fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_username_trims_and_lowercases() {
        assert_eq!(normalize_username("  BRadmin "), "bradmin");
        assert_eq!(normalize_username("bradmin"), "bradmin");
    }
}
