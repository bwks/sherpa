use std::process::Command;

use anyhow::Result;

use crate::core::konst::{SHERPA_SSH_CONFIG_FILE, TEMP_DIR};
use crate::libvirt::get_mgmt_ip;
use crate::libvirt::Qemu;
use crate::util::term_msg_surround;

pub fn ssh(qemu: &Qemu, name: &str, lab_name: &str, lab_id: &str) -> Result<()> {
    term_msg_surround(&format!("Connecting to: {name}"));

    let qemu_conn = qemu.connect()?;

    if let Some(vm_ip) = get_mgmt_ip(&qemu_conn, &format!("{}-{}-{}", name, lab_name, lab_id))? {
        let status = Command::new("ssh")
            .arg(&vm_ip)
            .arg("-F")
            .arg(format!("{TEMP_DIR}/{SHERPA_SSH_CONFIG_FILE}"))
            .status()?;

        if !status.success() {
            eprintln!("SSH connection failed");
            if let Some(code) = status.code() {
                eprintln!("Exit code: {}", code);
            }
        }
    } else {
        eprintln!("No IP address found for {name}")
    }
    Ok(())
}

pub fn ssh2(name: &str) -> Result<()> {
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
