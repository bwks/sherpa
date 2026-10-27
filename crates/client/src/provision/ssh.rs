//! The SSH provisioner: copies files and env files, then runs scripts, as the node's login
//! user, over one SSH connection per node.
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use shared::konst::{PROVISIONER_FILE_DEFAULT_MODE, PROVISIONER_SSH_RETRY_INTERVAL};
use tokio::time;
use topology::SshProvisioner;

use super::client_files::{existing_client_file, resolve_client_path};
use super::remote::{
    cloud_init_wait_command, env_file_destination, env_loader_command, finish_file_command,
    prepare_file_command, script_wrapper_command, sftp_path,
};
use super::transport::{NodeSession, SshTarget};
use crate::ssh_client::retryable;

/// A client file to copy, read before the node is changed.
struct FileCopy {
    src: PathBuf,
    dst: String,
    mode: u32,
}

/// A client script to run, read before the node is changed.
struct Script {
    path: PathBuf,
    content: Vec<u8>,
}

/// Connect once the node accepts the login user, then wait until cloud-init has finished
/// first-boot work. sshd can answer before cloud-init has created the user, so failures at the
/// node are retried until `timeout`; a failure at the Sherpa server, such as a refused login
/// or host key, is not.
pub async fn wait_for_node(target: &SshTarget, timeout: Duration) -> Result<NodeSession> {
    let deadline = Instant::now() + timeout;
    let session = loop {
        match target.connect().await {
            Ok(session) => break session,
            Err(error) if !retryable(&error, &target.node()) => return Err(error),
            Err(error) if Instant::now() >= deadline => {
                return Err(error.context(format!(
                    "{} did not accept the login within {}s",
                    target.host(),
                    timeout.as_secs()
                )));
            }
            Err(error) => {
                tracing::debug!(host = %target.host(), error = %format!("{error:#}"), "Node not accepting the login yet");
                time::sleep(Duration::from_secs(PROVISIONER_SSH_RETRY_INTERVAL)).await;
            }
        }
    };
    session
        .run(&cloud_init_wait_command())
        .await
        .with_context(|| format!("Cannot wait for cloud-init on {}", session.host()))?;
    Ok(session)
}

/// Run one SSH provisioner on a node: files, then env files, then scripts.
pub async fn provision(
    session: &NodeSession,
    provisioner: &SshProvisioner,
    manifest_dir: &Path,
) -> Result<()> {
    let (files, env_files, scripts) = read_client_files(provisioner, manifest_dir)?;

    for file in &files {
        println!("  copying {} to {}", file.src.display(), file.dst);
        copy_file(session, file).await?;
    }
    if !env_files.is_empty() {
        for file in &env_files {
            println!("  copying env file {} to {}", file.src.display(), file.dst);
            copy_file(session, file).await?;
        }
        session
            .run(&env_loader_command())
            .await
            .context("Cannot make login shells load env files")?;
    }
    for script in &scripts {
        println!("  running {}", script.path.display());
        session
            .run_with_input(&script_wrapper_command(), &script.content)
            .await
            .with_context(|| format!("Script {} failed", script.path.display()))?;
    }
    Ok(())
}

/// Resolve and read every client file the provisioner uses, so a missing file fails before the
/// node is changed.
fn read_client_files(
    provisioner: &SshProvisioner,
    manifest_dir: &Path,
) -> Result<(Vec<FileCopy>, Vec<FileCopy>, Vec<Script>)> {
    let mut files = Vec::with_capacity(provisioner.files.len());
    for file in &provisioner.files {
        let src = existing_client_file(manifest_dir, &file.src)?;
        files.push(FileCopy {
            src,
            dst: file.dst.clone(),
            mode: file.mode,
        });
    }

    let mut env_files = Vec::with_capacity(provisioner.env_files.len());
    for env_file in &provisioner.env_files {
        let src = existing_client_file(manifest_dir, env_file)?;
        let name = src
            .file_name()
            .and_then(OsStr::to_str)
            .with_context(|| format!("Env file {} does not name a file", src.display()))?
            .to_owned();
        env_files.push(FileCopy {
            src,
            dst: env_file_destination(&name),
            mode: PROVISIONER_FILE_DEFAULT_MODE,
        });
    }

    let mut scripts = Vec::with_capacity(provisioner.scripts.len());
    for script in &provisioner.scripts {
        let path = resolve_client_path(manifest_dir, script);
        let content = fs::read(&path)
            .with_context(|| format!("Cannot read provisioner script {}", path.display()))?;
        scripts.push(Script { path, content });
    }

    Ok((files, env_files, scripts))
}

async fn copy_file(session: &NodeSession, file: &FileCopy) -> Result<()> {
    session
        .run(&prepare_file_command(&file.dst))
        .await
        .with_context(|| format!("Cannot prepare {}", file.dst))?;
    session.copy_to(&file.src, &sftp_path(&file.dst)).await?;
    session
        .run(&finish_file_command(&file.dst, file.mode))
        .await
        .with_context(|| format!("Cannot set the mode of {}", file.dst))
}

#[cfg(test)]
mod tests {
    use super::*;
    use topology::ProvisionerFile;

    use tempfile::tempdir;

    #[test]
    fn client_files_are_read_with_their_node_destinations() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("token"), "t").unwrap();
        fs::write(dir.path().join("dev.env"), "A=1\n").unwrap();
        fs::write(dir.path().join("setup.sh"), "#!/bin/sh\n").unwrap();
        let provisioner = SshProvisioner {
            files: vec![ProvisionerFile {
                src: "token".to_string(),
                dst: "~/.config/token".to_string(),
                mode: 640,
            }],
            env_files: vec!["dev.env".to_string()],
            scripts: vec!["setup.sh".to_string()],
        };
        let (files, env_files, scripts) = read_client_files(&provisioner, dir.path()).unwrap();
        assert_eq!(files[0].src, dir.path().join("token"));
        assert_eq!(files[0].dst, "~/.config/token");
        assert_eq!(files[0].mode, 640);
        assert_eq!(env_files[0].dst, "~/.env.d/dev.env");
        assert_eq!(env_files[0].mode, 600);
        assert_eq!(scripts[0].content, b"#!/bin/sh\n");
    }

    #[test]
    fn a_missing_client_file_is_reported_by_name() {
        let dir = tempdir().unwrap();
        let provisioner = SshProvisioner {
            env_files: vec!["missing.env".to_string()],
            ..Default::default()
        };
        let Err(error) = read_client_files(&provisioner, dir.path()) else {
            panic!("a missing env file was read");
        };
        let error = format!("{error:#}");
        assert!(error.contains("missing.env"), "{error}");
    }
}
