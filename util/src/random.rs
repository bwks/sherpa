use anyhow::Result;
use twox_hash::XxHash32;

use super::user::get_username;

/// Get the xxHash of a username and current working directory
pub fn get_id(lab_name: &str) -> Result<String> {
    let combined = format!("{}{}", get_username()?, lab_name);
    let seed = 4_294_967_295;
    let hash = XxHash32::oneshot(seed, combined.as_bytes());
    Ok(format!("{:x}", hash))
}
