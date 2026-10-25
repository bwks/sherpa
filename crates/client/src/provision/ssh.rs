//! The SSH provisioner: copies files and env files with `scp`, then runs scripts, as the node's
//! login user.
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};

use shared::konst::{PROVISIONER_FILE_DEFAULT_MODE, PROVISIONER_SSH_RETRY_INTERVAL};
use shared::util::expand_path;
use topology::SshProvisioner;

use super::remote::{
    cloud_init_wait_command, env_file_destination, env_loader_command, finish_file_command,
    prepare_file_command, scp_path, script_wrapper_command,
};
use super::transport::SshTarget;

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

/// Wait until the node accepts the login user, then until cloud-init has finished first-boot
/// work. sshd can answer before cloud-init has created the user.
pub fn wait_for_node(target: &SshTarget, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        match target.run("true") {
            Ok(()) => break,
            Err(error) if Instant::now() >= deadline => {
                return Err(error.context(format!(
                    "{} did not accept the login within {}s",
                    target.host(),
                    timeout.as_secs()
                )));
            }
            Err(error) => {
                tracing::debug!(host = %target.host(), error = %format!("{error:#}"), "Node not accepting the login yet");
                thread::sleep(Duration::from_secs(PROVISIONER_SSH_RETRY_INTERVAL));
            }
        }
    }
    target
        .run(&cloud_init_wait_command())
        .with_context(|| format!("Cannot wait for cloud-init on {}", target.host()))
}

/// Run one SSH provisioner on a node: files, then env files, then scripts.
pub fn provision(
    target: &SshTarget,
    provisioner: &SshProvisioner,
    manifest_dir: &Path,
) -> Result<()> {
    let (files, env_files, scripts) = read_client_files(provisioner, manifest_dir)?;

    for file in &files {
        println!("  copying {} to {}", file.src.display(), file.dst);
        copy_file(target, file)?;
    }
    if !env_files.is_empty() {
        for file in &env_files {
            println!("  copying env file {} to {}", file.src.display(), file.dst);
            copy_file(target, file)?;
        }
        target
            .run(&env_loader_command())
            .context("Cannot make login shells load env files")?;
    }
    for script in &scripts {
        println!("  running {}", script.path.display());
        target
            .run_with_input(&script_wrapper_command(), &script.content)
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

/// Resolve a client path and check that it is a readable file.
fn existing_client_file(manifest_dir: &Path, path: &str) -> Result<PathBuf> {
    let resolved = resolve_client_path(manifest_dir, path);
    let metadata = fs::metadata(&resolved)
        .with_context(|| format!("Cannot read provisioner file {}", resolved.display()))?;
    ensure!(
        metadata.is_file(),
        "Provisioner file {} is not a file",
        resolved.display()
    );
    Ok(resolved)
}

/// A client path: `~` is expanded, and relative paths start at the manifest's directory.
fn resolve_client_path(manifest_dir: &Path, path: &str) -> PathBuf {
    let expanded = PathBuf::from(expand_path(path));
    if expanded.is_absolute() {
        expanded
    } else {
        manifest_dir.join(expanded)
    }
}

fn copy_file(target: &SshTarget, file: &FileCopy) -> Result<()> {
    target
        .run(&prepare_file_command(&file.dst))
        .with_context(|| format!("Cannot prepare {}", file.dst))?;
    target.copy_to(&file.src, &scp_path(&file.dst))?;
    target
        .run(&finish_file_command(&file.dst, file.mode))
        .with_context(|| format!("Cannot set the mode of {}", file.dst))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use topology::ProvisionerFile;

    use tempfile::tempdir;

    #[test]
    fn client_paths_start_at_the_manifest_unless_absolute() {
        let dir = Path::new("/labs/dev");
        assert_eq!(
            resolve_client_path(dir, "secrets/a.env"),
            Path::new("/labs/dev/secrets/a.env")
        );
        let absolute = env::temp_dir().join("a");
        assert_eq!(
            resolve_client_path(dir, absolute.to_str().unwrap()),
            absolute
        );
        let home = expand_path("~");
        assert_eq!(
            resolve_client_path(dir, "~/a.env"),
            Path::new(&home).join("a.env")
        );
    }

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
    fn a_missing_client_file_fails_before_connecting() {
        let dir = tempdir().unwrap();
        let provisioner = SshProvisioner {
            env_files: vec!["missing.env".to_string()],
            ..Default::default()
        };
        let target = SshTarget::new(Path::new("/nonexistent/sherpa_ssh_config"), "dev01.lab");
        let error = format!(
            "{:#}",
            provision(&target, &provisioner, dir.path()).unwrap_err()
        );
        assert!(error.contains("missing.env"), "{error}");
        assert!(!error.contains("ssh"), "{error}");
    }
}
