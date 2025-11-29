use anyhow::{Result, anyhow};

pub fn split_dev_int(text: &String) -> Result<(String, String)> {
    let (device, interface) = text
        .split_once("::")
        .ok_or_else(|| anyhow!("Missing :: in {}", text))?;

    Ok((device.to_string(), interface.to_string()))
}
