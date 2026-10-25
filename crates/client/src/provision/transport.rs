//! SSH transport for provisioners: the only code that runs `ssh` and `scp`.
//!
//! It uses the lab's `sherpa_ssh_config`, so connections go through the server jump host with
//! the node's login user, the same as `sherpa ssh`. Its two operations, running a command and
//! copying a file, match `lib_sshinto`'s `Session` and `Connection`, which can replace the
//! system binaries here.
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use shared::konst::PROVISIONER_SSH_CONNECT_TIMEOUT;

/// A node reached through the lab's SSH config.
pub struct SshTarget {
    config: PathBuf,
    host: String,
}

impl SshTarget {
    pub fn new(config: &Path, host: &str) -> Self {
        Self {
            config: config.to_path_buf(),
            host: host.to_owned(),
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// Run `command` on the node, keeping its output to report a failure.
    pub fn run(&self, command: &str) -> Result<()> {
        let output = Command::new("ssh")
            .args(self.ssh_args(command))
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("Cannot run ssh to {}", self.host))?;
        if !output.status.success() {
            bail!(
                "{} on {}: {}",
                exit_description(output.status.code()),
                self.host,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    /// Run `command` on the node with `input` on its stdin, showing its output as it runs.
    pub fn run_with_input(&self, command: &str, input: &[u8]) -> Result<()> {
        let mut child = Command::new("ssh")
            .args(self.ssh_args(command))
            .stdin(Stdio::piped())
            .spawn()
            .with_context(|| format!("Cannot run ssh to {}", self.host))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input)
                .with_context(|| format!("Cannot send input to {}", self.host))?;
        }
        let status = child
            .wait()
            .with_context(|| format!("Cannot wait for ssh to {}", self.host))?;
        if !status.success() {
            bail!("{} on {}", exit_description(status.code()), self.host);
        }
        Ok(())
    }

    /// Copy the client file `src` to `dst` on the node, an `scp` path.
    pub fn copy_to(&self, src: &Path, dst: &str) -> Result<()> {
        let output = Command::new("scp")
            .args(self.scp_args(src, dst))
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("Cannot run scp to {}", self.host))?;
        if !output.status.success() {
            bail!(
                "scp of {} to {}:{dst} failed: {}",
                src.display(),
                self.host,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    /// The lab's config, and no prompts: a connection fails rather than asking for a password
    /// or passphrase.
    fn common_args(&self) -> Vec<OsString> {
        vec![
            "-F".into(),
            self.config.clone().into_os_string(),
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            format!("ConnectTimeout={PROVISIONER_SSH_CONNECT_TIMEOUT}").into(),
            // Keep notices such as "Permanently added ... to known hosts" out of script output.
            "-o".into(),
            "LogLevel=ERROR".into(),
        ]
    }

    fn ssh_args(&self, command: &str) -> Vec<OsString> {
        let mut args = self.common_args();
        args.push("-T".into());
        args.push(self.host.clone().into());
        args.push(command.into());
        args
    }

    fn scp_args(&self, src: &Path, dst: &str) -> Vec<OsString> {
        let mut args = self.common_args();
        args.push("-q".into());
        args.push(src.as_os_str().to_os_string());
        args.push(format!("{}:{dst}", self.host).into());
        args
    }
}

fn exit_description(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("exit status {code}"),
        None => "terminated by a signal".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: Vec<OsString>) -> Vec<String> {
        let mut out = Vec::new();
        for arg in args {
            out.push(arg.to_string_lossy().into_owned());
        }
        out
    }

    #[test]
    fn ssh_uses_the_lab_config_without_prompts_and_runs_one_command() {
        let target = SshTarget::new(Path::new("/lab/sherpa_ssh_config"), "dev01.abcd1234");
        assert_eq!(
            strings(target.ssh_args("echo hi")),
            vec![
                "-F",
                "/lab/sherpa_ssh_config",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=15",
                "-o",
                "LogLevel=ERROR",
                "-T",
                "dev01.abcd1234",
                "echo hi",
            ]
        );
    }

    #[test]
    fn scp_copies_one_file_to_the_node_quietly() {
        let target = SshTarget::new(Path::new("/lab/sherpa_ssh_config"), "dev01.abcd1234");
        assert_eq!(
            strings(target.scp_args(Path::new("/home/me/token"), ".config/token")),
            vec![
                "-F",
                "/lab/sherpa_ssh_config",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=15",
                "-o",
                "LogLevel=ERROR",
                "-q",
                "/home/me/token",
                "dev01.abcd1234:.config/token",
            ]
        );
    }
}
