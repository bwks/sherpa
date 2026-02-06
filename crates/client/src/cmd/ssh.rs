use std::process::Command;

use anyhow::Result;

use shared::konst::{SHERPA_BASE_DIR, SHERPA_LABS_DIR, SHERPA_SSH_CONFIG_FILE};
use shared::util::term_msg_surround;

pub async fn ssh(lab_id: &str, name: &str) -> Result<()> {
    term_msg_surround(&format!("Connecting to: {name}"));
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");

    let status = Command::new("ssh")
        .arg(name)
        .arg("-F")
        .arg(format!("{lab_dir}/{SHERPA_SSH_CONFIG_FILE}"))
        .status()?;

    if !status.success() {
        eprintln!("SSH connection failed");
        if let Some(code) = status.code() {
            eprintln!("Exit code: {}", code);
        }
    }
    Ok(())
}
