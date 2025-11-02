use std::env;

use anyhow::{anyhow, Result};

/// Get the username of the current user from environment variables.
pub fn get_username() -> Result<String> {
    env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .map_err(|_| anyhow!("Failed to get current user from environment variables"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_get_username_from_user() {
        env::set_var("USER", "testuser");
        env::remove_var("USERNAME");
        let username = get_username().unwrap();
        assert_eq!(username, "testuser");
        env::remove_var("USER");
    }

    #[test]
    fn test_get_username_from_username() {
        env::remove_var("USER");
        env::set_var("USERNAME", "testuser");
        let username = get_username().unwrap();
        assert_eq!(username, "testuser");
        env::remove_var("USERNAME");
    }

    #[test]
    fn test_get_username_failure() {
        env::remove_var("USER");
        env::remove_var("USERNAME");
        let result = get_username();
        assert!(result.is_err());
    }
}
