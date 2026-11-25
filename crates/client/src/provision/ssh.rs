//! The SSH provisioner: copies files and env files, then runs scripts, as the node's login
//! user, over one SSH connection per node.
use std::ffi::OsStr;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use shared::konst::{PROVISIONER_FILE_DEFAULT_MODE, PROVISIONER_SSH_RETRY_INTERVAL};
use shared::validators::mode_from_octal_digits;
use tokio::time;
use topology::{Provisioner, SshProvisioner};

use super::client_files::{existing_client_file, resolve_client_path};
use super::env_file::{client_variable, resolve_env_file};
use super::output::NodeOutput;
use super::remote::{
    cloud_init_wait_command, env_file_destination, env_loader_command, parent_folders_command,
    script_wrapper_command, sftp_path,
};
use super::transport::{NodeSession, SshTarget};
use crate::error::ClientError;
use crate::ssh_client::retryable;

/// A client file to copy, read before the node is changed.
struct FileCopy {
    src: PathBuf,
    content: Vec<u8>,
    dst: String,
    mode: u32,
}

/// A client script to run, read before the node is changed.
struct Script {
    path: PathBuf,
    content: Vec<u8>,
}

/// What one ssh provisioner puts on a node, read from the client before the node is changed.
pub(super) struct ClientFiles {
    files: Vec<FileCopy>,
    env_files: Vec<FileCopy>,
    scripts: Vec<Script>,
}

/// Connect once the node accepts the login user, then wait until cloud-init has finished
/// first-boot work, both within `timeout`. sshd can answer before cloud-init has created the
/// user, so failures at the node are retried until `timeout`; a failure at the Sherpa server,
/// such as a refused login or host key, is not.
#[tracing::instrument(level = "debug", skip_all, fields(host = %target.host()))]
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
    let remaining = deadline.saturating_duration_since(Instant::now());
    let waited = within_ready_timeout(
        session.run(&cloud_init_wait_command()),
        remaining,
        session.host(),
        timeout,
    )
    .await;
    match waited {
        Ok(()) => Ok(session),
        Err(error) => {
            session.close().await;
            Err(error)
        }
    }
}

/// Wait for cloud-init on `host` with `wait`, giving up once `remaining` of the ready
/// `timeout` has passed.
async fn within_ready_timeout(
    wait: impl Future<Output = Result<()>>,
    remaining: Duration,
    host: &str,
    timeout: Duration,
) -> Result<()> {
    match time::timeout(remaining, wait).await {
        Ok(waited) => waited.with_context(|| format!("Cannot wait for cloud-init on {host}")),
        Err(_) => Err(ClientError::CloudInitTimeout {
            host: host.to_owned(),
            timeout_secs: timeout.as_secs(),
        }
        .into()),
    }
}

/// Read the client files of every ssh provisioner of a node, in manifest order, so a missing
/// file or unset variable in any of them fails the node before it is changed.
///
/// # Errors
///
/// The first client file that cannot be read, or env file variable that is not set.
#[tracing::instrument(level = "debug", skip_all)]
pub(super) fn read_node_provisioners(
    provisioners: &[Provisioner],
    manifest_dir: &Path,
) -> Result<Vec<ClientFiles>> {
    let mut read = Vec::with_capacity(provisioners.len());
    for provisioner in provisioners {
        if let Provisioner::Ssh(ssh) = provisioner {
            read.push(read_client_files(ssh, manifest_dir)?);
        }
    }
    Ok(read)
}

/// Run one SSH provisioner on a node: files, then env files, then scripts, showing progress
/// and script output in `output`.
#[tracing::instrument(level = "debug", skip_all)]
pub async fn provision(
    session: &NodeSession,
    client_files: &ClientFiles,
    output: &mut NodeOutput,
) -> Result<()> {
    let ClientFiles {
        files,
        env_files,
        scripts,
    } = client_files;

    for file in files {
        output.status(&format!("copying {} to {}", file.src.display(), file.dst));
        copy_file(session, file).await?;
    }
    if !env_files.is_empty() {
        for file in env_files {
            output.status(&format!(
                "copying env file {} to {}",
                file.src.display(),
                file.dst
            ));
            copy_file(session, file).await?;
        }
        session
            .run(&env_loader_command())
            .await
            .context("Cannot make login shells load env files")?;
    }
    for script in scripts {
        output.status(&format!("running {}", script.path.display()));
        session
            .run_with_input(&script_wrapper_command(), &script.content, output)
            .await
            .with_context(|| format!("Script {} failed", script.path.display()))?;
    }
    Ok(())
}

/// Resolve and read every client file the provisioner uses, and resolve the variable
/// references in env files, so a missing file or variable fails before the node is changed.
fn read_client_files(provisioner: &SshProvisioner, manifest_dir: &Path) -> Result<ClientFiles> {
    let mut files = Vec::with_capacity(provisioner.files.len());
    for file in &provisioner.files {
        let src = existing_client_file(manifest_dir, &file.src)?;
        let content = fs::read(&src).with_context(|| format!("Cannot read {}", src.display()))?;
        files.push(FileCopy {
            src,
            content,
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
        let text = fs::read_to_string(&src)
            .with_context(|| format!("Cannot read env file {} as text", src.display()))?;
        let content = resolve_env_file(&text, client_variable)
            .with_context(|| ClientError::EnvFile { path: src.clone() })?
            .into_bytes();
        env_files.push(FileCopy {
            src,
            content,
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

    Ok(ClientFiles {
        files,
        env_files,
        scripts,
    })
}

async fn copy_file(session: &NodeSession, file: &FileCopy) -> Result<()> {
    let mode = mode_from_octal_digits(file.mode)
        .with_context(|| format!("Provisioner file {} mode is invalid", file.dst))?;
    session
        .run(&parent_folders_command(&file.dst))
        .await
        .with_context(|| format!("Cannot create the folders of {}", file.dst))?;
    session
        .copy_to(&file.src, &file.content, &sftp_path(&file.dst), mode)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::future;

    use jato::{PromptCache, SshConfig};
    use shared::data::ClientConfig;
    use topology::ProvisionerFile;

    use tempfile::tempdir;

    use crate::ssh_client::{HostKeyRefresh, SshAccess};

    #[test]
    fn provisioner_files_002_every_client_file_is_read_with_where_it_goes_before_the_node_is_changed()
     {
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
        let ClientFiles {
            files,
            env_files,
            scripts,
        } = read_client_files(&provisioner, dir.path()).unwrap();
        assert_eq!(files[0].src, dir.path().join("token"));
        assert_eq!(files[0].content, b"t");
        assert_eq!(files[0].dst, "~/.config/token");
        assert_eq!(files[0].mode, 640);
        assert_eq!(env_files[0].dst, "~/.env.d/dev.env");
        assert_eq!(env_files[0].mode, 600);
        assert_eq!(scripts[0].content, b"#!/bin/sh\n");
    }

    #[test]
    fn provisioner_env_009_the_value_is_read_from_the_environment_sherpa_runs_in() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("dev.env"),
            "WHERE=$PATH\nPATH=$HOME/bin:$PATH\n",
        )
        .unwrap();
        let provisioner = SshProvisioner {
            env_files: vec!["dev.env".to_string()],
            ..Default::default()
        };
        let env_files = read_client_files(&provisioner, dir.path())
            .unwrap()
            .env_files;
        // PATH is set on every client platform, and holds no single quote; HOME is not set on
        // Windows.
        let path = env::var("PATH").unwrap();
        assert_eq!(
            String::from_utf8(env_files[0].content.clone()).unwrap(),
            format!("WHERE='{path}'\nPATH=$HOME/bin:$PATH\n")
        );
    }

    /// A node's provisioners: a first one whose script is there, then `second`.
    fn after_a_good_provisioner(dir: &Path, second: SshProvisioner) -> Vec<Provisioner> {
        fs::write(dir.join("first.sh"), "#!/bin/sh\n").unwrap();
        vec![
            Provisioner::Ssh(SshProvisioner {
                scripts: vec!["first.sh".to_string()],
                ..Default::default()
            }),
            Provisioner::Ssh(second),
        ]
    }

    #[test]
    fn provisioner_env_007_an_unset_variable_in_any_provisioner_fails_before_the_node_is_changed_carrying_file_line_and_variable()
     {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("dev.env"),
            "A=1\nTOKEN=$SHERPA_TEST_UNSET_TOKEN\n",
        )
        .unwrap();
        let provisioners = after_a_good_provisioner(
            dir.path(),
            SshProvisioner {
                env_files: vec!["dev.env".to_string()],
                ..Default::default()
            },
        );
        let Err(error) = read_node_provisioners(&provisioners, dir.path()) else {
            panic!("an env file with an unset variable was read");
        };
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::EnvFile { path }) if *path == dir.path().join("dev.env")
            ),
            "{error:#}"
        );
        assert!(
            matches!(
                error.root_cause().downcast_ref::<ClientError>(),
                Some(ClientError::UnsetEnvFileVariable { line: 2, variable })
                    if variable == "SHERPA_TEST_UNSET_TOKEN"
            ),
            "{error:#}"
        );
    }

    #[test]
    fn provisioner_files_003_a_missing_client_file_in_any_provisioner_fails_the_node_carrying_it() {
        let dir = tempdir().unwrap();
        let provisioners = after_a_good_provisioner(
            dir.path(),
            SshProvisioner {
                env_files: vec!["missing.env".to_string()],
                ..Default::default()
            },
        );
        let Err(error) = read_node_provisioners(&provisioners, dir.path()) else {
            panic!("a missing env file was read");
        };
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::ProvisionerFileUnreadable { path })
                    if *path == dir.path().join("missing.env")
            ),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn provision_run_015_a_node_whose_cloud_init_has_not_finished_within_the_ready_timeout_fails()
     {
        let error = within_ready_timeout(
            future::pending(),
            Duration::from_millis(10),
            "web01.ab12cd34",
            Duration::from_secs(600),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::CloudInitTimeout { host, timeout_secs: 600 })
                    if host == "web01.ab12cd34"
            ),
            "{error:#}"
        );
        within_ready_timeout(
            async { Ok(()) },
            Duration::ZERO,
            "web01.ab12cd34",
            Duration::from_secs(600),
        )
        .await
        .unwrap();
    }

    /// A target for `web01` of lab `ab12cd34`, whose SSH config puts it at `address`, reached
    /// with no jump host.
    fn direct_target(dir: &Path, address: &str) -> SshTarget {
        let path = dir.join("config");
        fs::write(
            &path,
            format!(
                "Host {address} web01.ab12cd34\n    HostName {address}\n    User alice\n    \
                 StrictHostKeyChecking no\n    UserKnownHostsFile /dev/null\n"
            ),
        )
        .unwrap();
        let access = SshAccess {
            refresh: HostKeyRefresh::new(
                "ab12cd34",
                "wss://sherpa.example:3030/ws",
                &ClientConfig::default(),
                dir,
            ),
            jump: None,
        };
        SshTarget::new(
            &SshConfig::load_file(&path).unwrap(),
            "web01",
            "ab12cd34",
            &PromptCache::new(),
            &access,
        )
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "makes a real TCP connection attempt to 192.0.2.10 (TEST-NET-1, never routed)"]
    async fn ssh_access_006_an_unreachable_node_fails_at_once_with_a_hint() {
        let dir = tempdir().unwrap();
        let target = direct_target(dir.path(), "192.0.2.10");
        let started = Instant::now();
        let Err(error) = wait_for_node(&target, Duration::from_secs(600)).await else {
            panic!("a node with no route was reached");
        };
        let waited = started.elapsed();
        assert!(
            waited < Duration::from_secs(60),
            "waited {waited:?} before failing: {error:#}"
        );
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::NoRouteToNode { node_name, address, .. })
                    if node_name == "web01" && address == "192.0.2.10:22"
            ),
            "{error:#}"
        );
    }
}
