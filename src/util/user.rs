use std::env;

use anyhow::{anyhow, Result};

#[allow(dead_code)]
/// Get the username of the current user from environment variables.
pub fn get_username() -> Result<String> {
    env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .map_err(|_| anyhow!("Failed to get current user from environment variables"))
}
