use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use jato::{ConnectOptions, InteractiveOptions, PromptCache, SshConfig};
use topology::Manifest;

use shared::data::{ClientConfig, LabInfo};
use shared::konst::{SHERPA_DOMAIN_NAME, SHERPA_MANIFEST_FILE, SSH_PROMPT_AUTH_TIMEOUT};
use shared::util::{LabFolder, term_msg_surround};

use super::lab_identity::require_lab_owner;
use crate::error::ClientError;
use crate::ssh_client::{self, PromptScope, SshAccess, node_route};

/// Qualify a device name with the lab ID suffix if not already present.
///
/// SSH hosts in the sherpa config use the `<node>.<lab-id>` format.
/// If the user provides just the node name (e.g. `dev01`), this appends
/// the lab suffix so it becomes `dev01.a10736e8`.
fn qualify_device_name(name: &str, lab_id: &str) -> String {
    let suffix = format!(".{lab_id}");
    if name.ends_with(&suffix) || name.ends_with(&format!("{suffix}.{SHERPA_DOMAIN_NAME}")) {
        name.to_string()
    } else {
        format!("{name}{suffix}")
    }
}

/// The node name in `name`, a node name or its alias in the lab SSH config.
fn node_name<'a>(name: &'a str, lab_id: &str) -> &'a str {
    let fully_qualified = format!(".{lab_id}.{SHERPA_DOMAIN_NAME}");
    let qualified = format!(".{lab_id}");
    match name.strip_suffix(&fully_qualified) {
        Some(node) => node,
        None => name.strip_suffix(&qualified).unwrap_or(name),
    }
}

/// The line on stderr once the shell is open, naming the escape.
fn connected_line(qualified_name: &str) -> String {
    format!("Connected to {qualified_name}. Escape: ~. at the start of a line.")
}

/// The line on stderr when the session ends. After `~.` the cursor is usually on the node's
/// prompt, so the line starts on a new line.
fn closed_line(qualified_name: &str, escaped: bool) -> String {
    let gap = if escaped { "\n" } else { "" };
    format!("{gap}Connection to {qualified_name} closed.")
}

/// The node's shell ended with this non-zero exit code; `sherpa ssh` exits with it too. Windows'
/// OpenSSH server reports none for an interactive `PowerShell`, so a Windows node gives 0.
#[derive(Debug)]
pub struct ShellExit(pub u8);

impl fmt::Display for ShellExit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "The node's shell exited with code {}", self.0)
    }
}

impl std::error::Error for ShellExit {}

/// Open an interactive shell on a lab node, as the lab's SSH configuration says: directly, or
/// through the Sherpa server when `ssh_jump` is set. Only the lab's owner may: anyone else is
/// stopped before anything connects. A non-zero exit code from the node's shell is returned as
/// a `ShellExit` error.
#[tracing::instrument(skip(client_config), fields(%name, %lab_id))]
pub async fn ssh(
    name: &str,
    lab_id: &str,
    server_url: &str,
    client_config: &ClientConfig,
    ssh_jump: bool,
) -> Result<()> {
    let lab_dir = Path::new("");
    require_lab_owner(lab_dir)?;
    let qualified_name = load_ssh_connection(name, lab_id, lab_dir)?;
    let config = SshConfig::load_file(LabFolder::new(lab_dir).ssh_config())
        .context("Failed to read the lab SSH config")?;
    let access = SshAccess::new(lab_id, server_url, client_config, lab_dir, ssh_jump)?;
    let route = node_route(&config, &qualified_name, access.jump.as_ref())?;
    term_msg_surround(&format!("Connecting to: {qualified_name}"));

    let mut options = ConnectOptions::default();
    options.auth_timeout = Duration::from_secs(SSH_PROMPT_AUTH_TIMEOUT);
    let session = ssh_client::connect(
        &route,
        &options,
        PromptScope::AllHops,
        &PromptCache::new(),
        node_name(name, lab_id),
        &access.refresh,
    )
    .await?;
    eprintln!("{}", connected_line(&qualified_name));
    // Sherpa prints its own lines, with the node's name, in place of Jato's.
    let mut interactive = InteractiveOptions::default();
    interactive.banner = false;
    let end = session
        .interactive_shell(&interactive)
        .await
        .context("The shell failed")?;
    let code = end.exit_code();
    if let Err(error) = session.close().await {
        tracing::debug!(%error, "Failed to close the SSH session cleanly");
    }
    eprintln!("{}", closed_line(&qualified_name, end.escaped));
    if code != 0 {
        return Err(ShellExit(u8::try_from(code).unwrap_or(1)).into());
    }
    Ok(())
}

fn load_ssh_connection(name: &str, lab_id: &str, lab_dir: &Path) -> Result<String> {
    // A local manifest can reject an unknown node even when no downloaded SSH
    // files exist yet. Download-only directories fall back to saved Host aliases.
    match fs::read_to_string(lab_dir.join(SHERPA_MANIFEST_FILE)) {
        Ok(contents) => {
            let manifest: Manifest =
                toml::from_str(&contents).context("Failed to parse lab manifest")?;
            let node_name = node_name(name, lab_id);
            ensure!(
                manifest.nodes.iter().any(|node| node.name == node_name),
                ClientError::NodeNotInManifest {
                    node: name.to_owned()
                }
            );
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Failed to load lab manifest"),
    }
    let folder = LabFolder::new(lab_dir);
    let lab_info: LabInfo = read_lab_connection_file(&folder.lab_info(), lab_id)?.parse()?;
    let ssh_config = read_lab_connection_file(&folder.ssh_config(), lab_id)?;
    prepare_ssh_connection(name, lab_id, &lab_info, &ssh_config)
}

pub(super) fn read_lab_connection_file(path: &Path, lab_id: &str) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            bail!(ClientError::LabConnectionFilesMissing {
                lab_id: lab_id.to_owned()
            });
        }
        Err(error) => Err(error).with_context(|| ClientError::LabConnectionFileUnreadable {
            path: path.to_owned(),
        }),
    }
}

fn prepare_ssh_connection(
    name: &str,
    lab_id: &str,
    lab_info: &LabInfo,
    ssh_config: &str,
) -> Result<String> {
    let qualified_name = qualify_device_name(name, lab_id);
    // Sherpa emits explicit Host aliases for every configured node.
    let configured = ssh_config.lines().any(|line| {
        let mut fields = line.split_ascii_whitespace();
        fields
            .next()
            .is_some_and(|directive| directive.eq_ignore_ascii_case("Host"))
            && fields
                .take_while(|field| !field.starts_with('#'))
                .any(|alias| alias == qualified_name)
    });
    ensure!(
        configured,
        ClientError::NodeNotInLab {
            node: name.to_owned(),
            lab_id: lab_id.to_owned(),
        }
    );
    ensure!(
        lab_info.id == lab_id,
        ClientError::ConnectionFilesOfAnotherLab
    );
    Ok(qualified_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Error as IoError;
    use tempfile::tempdir;

    /// Write `content` to `path`, making its folders.
    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn lab_info() -> LabInfo {
        "id='abcd1234'\nname='test'\nuser='alice'\nipv4_network='192.0.2.0/24'\nipv4_gateway='192.0.2.1'\nipv4_router='192.0.2.2'\n[[ssh_usernames]]\nnode_name='vm'\nusername='alice'".parse().unwrap()
    }

    #[test]
    fn ssh_target_002_a_node_not_in_the_manifest_is_refused_before_lab_files_are_needed() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(SHERPA_MANIFEST_FILE),
            "name='test'\nnodes=[{name='oma01',model='omarchy_linux'}]\n",
        )
        .unwrap();
        let error = load_ssh_connection("oma1", "abcd1234", dir.path()).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::NodeNotInManifest { node }) if node == "oma1"
            ),
            "{error:#}"
        );
    }

    #[test]
    fn ssh_target_005_a_downloaded_folder_connects_without_a_manifest() {
        let dir = tempdir().unwrap();
        write(
            &LabFolder::new(dir.path()).lab_info(),
            &lab_info().to_string(),
        );
        write(
            &LabFolder::new(dir.path()).ssh_config(),
            "Host vm.abcd1234 vm.abcd1234.sherpa.lab.local\n    User alice\n",
        );
        for (name, host) in [
            ("vm", "vm.abcd1234"),
            ("vm.abcd1234", "vm.abcd1234"),
            (
                "vm.abcd1234.sherpa.lab.local",
                "vm.abcd1234.sherpa.lab.local",
            ),
        ] {
            assert_eq!(
                load_ssh_connection(name, "abcd1234", dir.path()).unwrap(),
                host
            );
        }
    }

    #[test]
    fn ssh_target_004_a_folder_without_connection_files_says_how_to_get_them() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(SHERPA_MANIFEST_FILE),
            "name='test'\nnodes=[{name='vm',model='omarchy_linux'}]\n",
        )
        .unwrap();
        let check = |dir: &Path| {
            for name in ["vm", "vm.abcd1234", "vm.abcd1234.sherpa.lab.local"] {
                let error = load_ssh_connection(name, "abcd1234", dir).unwrap_err();
                assert!(
                    matches!(
                        error.downcast_ref::<ClientError>(),
                        Some(ClientError::LabConnectionFilesMissing { lab_id }) if lab_id == "abcd1234"
                    ),
                    "{error:#}"
                );
            }
        };

        check(dir.path());
        write(
            &LabFolder::new(dir.path()).lab_info(),
            &lab_info().to_string(),
        );
        check(dir.path());
    }

    #[test]
    fn ssh_target_007_a_connection_file_that_cannot_be_read_fails_with_its_cause() {
        let dir = tempdir().unwrap();
        let error = read_lab_connection_file(dir.path(), "abcd1234").unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::LabConnectionFileUnreadable { path }) if path == dir.path()
            ),
            "{error:#}"
        );
        assert!(error.downcast_ref::<IoError>().is_some());
    }

    #[test]
    fn ssh_target_001_a_node_is_taken_by_any_of_its_names() {
        let config = "Host 192.0.2.10 vm.abcd1234 vm.abcd1234.sherpa.lab.local\n    User alice\n";
        for (name, host) in [
            ("vm", "vm.abcd1234"),
            ("vm.abcd1234", "vm.abcd1234"),
            (
                "vm.abcd1234.sherpa.lab.local",
                "vm.abcd1234.sherpa.lab.local",
            ),
        ] {
            assert_eq!(
                prepare_ssh_connection(name, "abcd1234", &lab_info(), config).unwrap(),
                host
            );
        }
    }

    #[test]
    fn ssh_target_006_connection_files_of_another_lab_are_refused() {
        let error = prepare_ssh_connection(
            "vm",
            "ffff0000",
            &lab_info(),
            "Host vm.ffff0000\n    User alice\n",
        )
        .unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::ConnectionFilesOfAnotherLab)
            ),
            "{error:#}"
        );
    }

    #[test]
    fn ssh_target_003_a_node_with_no_host_in_the_ssh_config_is_refused_carrying_it() {
        for config in [
            "Host 192.0.2.10 router.abcd1234 router.abcd1234.sherpa.lab.local\n    User alice\n",
            "# Host vm.abcd1234\n",
            "Host router.abcd1234 # vm.abcd1234\n",
            "HostName vm.abcd1234\n",
            "Host vm01.abcd1234\n",
        ] {
            let error = prepare_ssh_connection("vm", "abcd1234", &lab_info(), config).unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<ClientError>(),
                    Some(ClientError::NodeNotInLab { node, lab_id })
                        if node == "vm" && lab_id == "abcd1234"
                ),
                "{error:#}"
            );
        }
    }

    #[test]
    fn ssh_target_009_the_close_line_starts_on_a_new_line_after_an_escape() {
        assert_eq!(
            closed_line("dev01.a10736e8", false),
            "Connection to dev01.a10736e8 closed."
        );
        assert_eq!(
            closed_line("dev01.a10736e8", true),
            "\nConnection to dev01.a10736e8 closed."
        );
    }
}
