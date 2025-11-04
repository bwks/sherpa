use std::process::Command;

use anyhow::Result;

use konst::{SHERPA_SSH_CONFIG_FILE, TEMP_DIR};
use util::term_msg_surround;

pub fn ssh(name: &str) -> Result<()> {
    term_msg_surround(&format!("Connecting to: {name}"));

    let status = Command::new("ssh")
        .arg(&name)
        .arg("-F")
        .arg(format!("{TEMP_DIR}/{SHERPA_SSH_CONFIG_FILE}"))
        .status()?;

    if !status.success() {
        eprintln!("SSH connection failed");
        if let Some(code) = status.code() {
            eprintln!("Exit code: {}", code);
        }
    }
    Ok(())
}
