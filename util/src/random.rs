use anyhow::Result;
use twox_hash::XxHash32;

use super::file_system::get_cwd;
use super::user::get_username;

/// Get the xxHash of a username and current working directory
pub fn get_id() -> Result<String> {
    let combined = format!("{}{}", get_username()?, get_cwd()?);
    let seed = 123454321;
    let hash = XxHash32::oneshot(seed, combined.as_bytes());
    Ok(format!("{:x}", hash))
}
