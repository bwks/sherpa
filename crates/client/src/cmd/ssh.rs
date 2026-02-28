use std::process::Command;

use anyhow::Result;

use shared::konst::SHERPA_SSH_CONFIG_FILE;
use shared::util::term_msg_surround;

pub async fn ssh(name: &str) -> Result<()> {
    term_msg_surround(&format!("Connecting to: {name}"));

    let status = Command::new("ssh")
        .arg(name)
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
