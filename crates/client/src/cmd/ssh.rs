use std::process::Command;

use anyhow::Result;

use shared::konst::SHERPA_SSH_CONFIG_FILE;
use shared::util::term_msg_surround;

/// Qualify a device name with the lab ID suffix if not already present.
///
/// SSH hosts in the sherpa config use the `<node>.<lab-id>` format.
/// If the user provides just the node name (e.g. `dev01`), this appends
/// the lab suffix so it becomes `dev01.a10736e8`.
fn qualify_device_name(name: &str, lab_id: &str) -> String {
    let suffix = format!(".{}", lab_id);
    if name.ends_with(&suffix) {
        name.to_string()
    } else {
        format!("{}{}", name, suffix)
    }
}

pub async fn ssh(name: &str, lab_id: &str) -> Result<()> {
    let qualified_name = qualify_device_name(name, lab_id);
    term_msg_surround(&format!("Connecting to: {qualified_name}"));

    let status = Command::new("ssh")
        .arg(&qualified_name)
        .arg("-F")
        .arg(SHERPA_SSH_CONFIG_FILE)
        .status()?;

    if !status.success() {
        eprintln!("SSH connection failed");
        if let Some(code) = status.code() {
            eprintln!("Exit code: {}", code);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qualify_device_name_adds_suffix() {
        let result = qualify_device_name("dev01", "a10736e8");
        assert_eq!(result, "dev01.a10736e8");
    }

    #[test]
    fn test_qualify_device_name_already_qualified() {
        let result = qualify_device_name("dev01.a10736e8", "a10736e8");
        assert_eq!(result, "dev01.a10736e8");
    }

    #[test]
    fn test_qualify_device_name_different_suffix_not_matching() {
        // Name ends with a different lab id — should still append
        let result = qualify_device_name("dev01.deadbeef", "a10736e8");
        assert_eq!(result, "dev01.deadbeef.a10736e8");
    }

    #[test]
    fn test_qualify_device_name_partial_match_not_fooled() {
        // Name contains the lab id but not as a proper suffix
        let result = qualify_device_name("a10736e8-dev01", "a10736e8");
        assert_eq!(result, "a10736e8-dev01.a10736e8");
    }
}
