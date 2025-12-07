use std::process::Command;

use anyhow::Result;

use data::Config;
use konst::{SHERPA_SSH_CONFIG_FILE, TEMP_DIR};
use util::term_msg_surround;

pub async fn ssh(name: &str, config: &Config) -> Result<()> {
    term_msg_surround(&format!("Connecting to: {name}"));

    // let leases = get_dhcp_leases(config).await?;

    // let vm_ip = if let Some(vm_ip) = leases.iter().find(|d| d.hostname == name) {
    //     vm_ip.ipv4_address.clone()
    // } else {
    //     bail!("Unable to find IP address for: {name}")
    // };

    let status = Command::new("ssh")
        .arg(name)
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
