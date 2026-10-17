use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use topology::Manifest;

use shared::data::LabInfo;
use shared::konst::{
    LAB_FILE_NAME, SHERPA_DOMAIN_NAME, SHERPA_MANIFEST_FILE, SHERPA_SSH_CONFIG_FILE,
    SHERPA_SSH_PRIVATE_KEY_FILE,
};
use shared::util::term_msg_surround;

use crate::private_key::{SSH_PRIVATE_KEY_ERROR_LABEL, restrict_secret_permissions};

/// Qualify a device name with the lab ID suffix if not already present.
///
/// SSH hosts in the sherpa config use the `<node>.<lab-id>` format.
/// If the user provides just the node name (e.g. `dev01`), this appends
/// the lab suffix so it becomes `dev01.a10736e8`.
fn qualify_device_name(name: &str, lab_id: &str) -> String {
    let suffix = format!(".{}", lab_id);
    if name.ends_with(&suffix) || name.ends_with(&format!("{suffix}.{SHERPA_DOMAIN_NAME}")) {
        name.to_string()
    } else {
        format!("{}{}", name, suffix)
    }
}

#[tracing::instrument(fields(%name, %lab_id))]
pub async fn ssh(name: &str, lab_id: &str) -> Result<()> {
    let qualified_name = load_ssh_connection(name, lab_id, Path::new("."))?;
    term_msg_surround(&format!("Connecting to: {qualified_name}"));

    let status = Command::new("ssh")
        .arg(&qualified_name)
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

fn load_ssh_connection(name: &str, lab_id: &str, lab_dir: &Path) -> Result<String> {
    // A local manifest can reject an unknown node even when no downloaded SSH
    // files exist yet. Download-only directories fall back to saved Host aliases.
    match fs::read_to_string(lab_dir.join(SHERPA_MANIFEST_FILE)) {
        Ok(contents) => {
            let manifest: Manifest =
                toml::from_str(&contents).context("Failed to parse lab manifest")?;
            let node_name = unqualified_device_name(name, lab_id);
            ensure!(
                manifest.nodes.iter().any(|node| node.name == node_name),
                "Node '{name}' not found in {SHERPA_MANIFEST_FILE}"
            );
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Failed to load lab manifest"),
    }
    let lab_info: LabInfo =
        read_lab_connection_file(&lab_dir.join(LAB_FILE_NAME), lab_id)?.parse()?;
    let ssh_config = read_lab_connection_file(&lab_dir.join(SHERPA_SSH_CONFIG_FILE), lab_id)?;
    prepare_ssh_connection(
        name,
        lab_id,
        &lab_info,
        &ssh_config,
        &lab_dir.join(SHERPA_SSH_PRIVATE_KEY_FILE),
    )
}

fn read_lab_connection_file(path: &Path, lab_id: &str) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            bail!(
                "Lab connection files are missing. Start the lab with 'sherpa up', or restore existing lab files with 'sherpa download {lab_id} --force'."
            );
        }
        Err(error) => Err(error)
            .with_context(|| format!("Failed to read lab connection file: {}", path.display())),
    }
}

fn unqualified_device_name<'a>(name: &'a str, lab_id: &str) -> &'a str {
    name.strip_suffix(&format!(".{lab_id}.{SHERPA_DOMAIN_NAME}"))
        .or_else(|| name.strip_suffix(&format!(".{lab_id}")))
        .unwrap_or(name)
}

fn prepare_ssh_connection(
    name: &str,
    lab_id: &str,
    lab_info: &LabInfo,
    ssh_config: &str,
    key_path: &Path,
) -> Result<String> {
    let qualified_name = qualify_device_name(name, lab_id);
    // Sherpa emits explicit Host aliases for every configured node. Validate
    // against those aliases before treating a missing personal identity as legacy.
    let configured = ssh_config.lines().any(|line| {
        let mut fields = line.split_ascii_whitespace();
        fields
            .next()
            .is_some_and(|directive| directive.eq_ignore_ascii_case("Host"))
            && fields
                .take_while(|field| !field.starts_with('#'))
                .any(|alias| alias == qualified_name)
    });
    ensure!(configured, "Node '{name}' not found in lab '{lab_id}'");
    secure_legacy_key(name, lab_id, lab_info, key_path)?;
    Ok(qualified_name)
}

fn secure_legacy_key(name: &str, lab_id: &str, lab_info: &LabInfo, key_path: &Path) -> Result<()> {
    ensure!(
        lab_info.id == lab_id,
        "Saved SSH metadata belongs to a different lab"
    );
    let node_name = unqualified_device_name(name, lab_id);
    if lab_info.ssh_username(node_name).is_none() {
        restrict_secret_permissions(key_path, SSH_PRIVATE_KEY_ERROR_LABEL)
            .context("Failed to secure SSH private key before connecting")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Error as IoError;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn lab_info() -> LabInfo {
        "id='abcd1234'\nname='test'\nuser='alice'\nipv4_network='192.0.2.0/24'\nipv4_gateway='192.0.2.1'\nipv4_router='192.0.2.2'\nloopback_network='127.127.0.0/24'\n[[ssh_usernames]]\nnode_name='vm'\nusername='alice'".parse().unwrap()
    }

    #[test]
    fn unknown_manifest_node_is_reported_when_ssh_config_is_missing() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("manifest.toml"),
            "name='test'\nnodes=[{name='oma01',model='omarchy_linux'}]\n",
        )
        .unwrap();
        fs::write(dir.path().join(LAB_FILE_NAME), lab_info().to_string()).unwrap();
        let error = load_ssh_connection("oma1", "abcd1234", dir.path()).unwrap_err();
        assert_eq!(error.to_string(), "Node 'oma1' not found in manifest.toml");
    }

    #[test]
    fn unknown_manifest_node_does_not_require_lab_metadata_or_keys() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(SHERPA_MANIFEST_FILE),
            "name='test'\nnodes=[{name='oma01',model='omarchy_linux'}]\n",
        )
        .unwrap();
        let error = load_ssh_connection("oma1", "abcd1234", dir.path()).unwrap_err();
        assert_eq!(error.to_string(), "Node 'oma1' not found in manifest.toml");
    }

    #[test]
    fn downloaded_files_work_without_a_manifest() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(LAB_FILE_NAME), lab_info().to_string()).unwrap();
        fs::write(
            dir.path().join(SHERPA_SSH_CONFIG_FILE),
            "Host vm.abcd1234 vm.abcd1234.sherpa.lab.local\n    User alice\n",
        )
        .unwrap();
        for name in ["vm", "vm.abcd1234", "vm.abcd1234.sherpa.lab.local"] {
            assert_eq!(
                load_ssh_connection(name, "abcd1234", dir.path()).unwrap(),
                qualify_device_name(name, "abcd1234")
            );
        }
    }

    #[test]
    fn known_manifest_aliases_are_checked_before_loading_ssh_files() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(SHERPA_MANIFEST_FILE),
            "name='test'\nnodes=[{name='vm',model='omarchy_linux'}]\n",
        )
        .unwrap();
        fs::write(dir.path().join(LAB_FILE_NAME), lab_info().to_string()).unwrap();
        for name in ["vm", "vm.abcd1234", "vm.abcd1234.sherpa.lab.local"] {
            let error = load_ssh_connection(name, "abcd1234", dir.path()).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Lab connection files are missing"),
                "{error:#}"
            );
            assert!(error.to_string().contains("sherpa up"));
            assert!(
                error
                    .to_string()
                    .contains("sherpa download abcd1234 --force")
            );
        }
    }

    #[test]
    fn unstarted_lab_reports_how_to_create_its_connection_files() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(SHERPA_MANIFEST_FILE),
            "name='test'\nnodes=[{name='vm',model='omarchy_linux'}]\n",
        )
        .unwrap();
        let error = load_ssh_connection("vm", "abcd1234", dir.path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Lab connection files are missing"),
            "{error:#}"
        );
        assert!(error.to_string().contains("sherpa up"));
        assert!(
            error
                .to_string()
                .contains("sherpa download abcd1234 --force")
        );
    }

    #[test]
    fn connection_file_read_errors_keep_their_underlying_cause() {
        let dir = tempdir().unwrap();
        let error = read_lab_connection_file(dir.path(), "abcd1234").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Failed to read lab connection file")
        );
        assert!(!error.to_string().contains("sherpa up"));
        assert!(error.downcast_ref::<IoError>().is_some());
    }

    #[test]
    fn personal_nodes_do_not_require_sherpa_private_key() {
        let dir = tempdir().unwrap();
        let absent_key = dir.path().join("missing");
        for name in ["vm", "vm.abcd1234", "vm.abcd1234.sherpa.lab.local"] {
            secure_legacy_key(name, "abcd1234", &lab_info(), &absent_key).unwrap();
        }
        assert!(secure_legacy_key("router", "abcd1234", &lab_info(), &absent_key).is_err());
    }

    #[test]
    fn unknown_node_is_reported_before_missing_legacy_key() {
        let dir = tempdir().unwrap();
        let error = prepare_ssh_connection(
            "oma1",
            "abcd1234",
            &lab_info(),
            "Host 192.0.2.10 oma01.abcd1234 oma01.abcd1234.sherpa.lab.local\n    User alice\n",
            &dir.path().join("missing-key"),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("Node 'oma1' not found"),
            "{error:#}"
        );
    }

    #[test]
    fn configured_personal_host_aliases_do_not_require_a_legacy_key() {
        let dir = tempdir().unwrap();
        let config = "Host 192.0.2.10 vm.abcd1234 vm.abcd1234.sherpa.lab.local\n    User alice\n";
        for name in ["vm", "vm.abcd1234", "vm.abcd1234.sherpa.lab.local"] {
            assert_eq!(
                prepare_ssh_connection(
                    name,
                    "abcd1234",
                    &lab_info(),
                    config,
                    &dir.path().join("missing-key")
                )
                .unwrap(),
                qualify_device_name(name, "abcd1234")
            );
        }
    }

    #[test]
    fn configured_legacy_host_still_requires_its_key() {
        let dir = tempdir().unwrap();
        let error = prepare_ssh_connection(
            "router",
            "abcd1234",
            &lab_info(),
            "Host router.abcd1234\n",
            &dir.path().join("missing-key"),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Failed to secure SSH private key")
        );
    }

    #[test]
    fn comments_hostnames_and_partial_aliases_do_not_count_as_nodes() {
        let dir = tempdir().unwrap();
        for config in [
            "# Host vm.abcd1234\n",
            "Host router.abcd1234 # vm.abcd1234\n",
            "HostName vm.abcd1234\n",
            "Host vm01.abcd1234\n",
        ] {
            let error = prepare_ssh_connection(
                "vm",
                "abcd1234",
                &lab_info(),
                config,
                &dir.path().join("missing-key"),
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("Node 'vm' not found"),
                "{error:#}"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn legacy_nodes_still_enforce_private_key_permissions() {
        let dir = tempdir().unwrap();
        let key = dir.path().join("key");
        fs::write(&key, "test-key").unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
        secure_legacy_key("router", "abcd1234", &lab_info(), &key).unwrap();
        assert_eq!(
            fs::metadata(key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn fully_qualified_alias_is_preserved() {
        assert_eq!(
            qualify_device_name("vm.abcd1234.sherpa.lab.local", "abcd1234"),
            "vm.abcd1234.sherpa.lab.local"
        );
    }

    #[test]
    fn test_qualify_device_name_adds_suffix() {
        let result = qualify_device_name("dev01", "a10736e8");
        assert_eq!(result, "dev01.a10736e8");
    }

    #[test]
    fn test_qualify_device_name_already_qualified() {
        let result = qualify_device_name("dev01.a10736e8", "a10736e8");
        assert_eq!(result, "dev01.a10736e8");
    }

    #[test]
    fn test_qualify_device_name_different_suffix_not_matching() {
        // Name ends with a different lab id — should still append
        let result = qualify_device_name("dev01.deadbeef", "a10736e8");
        assert_eq!(result, "dev01.deadbeef.a10736e8");
    }

    #[test]
    fn test_qualify_device_name_partial_match_not_fooled() {
        // Name contains the lab id but not as a proper suffix
        let result = qualify_device_name("a10736e8-dev01", "a10736e8");
        assert_eq!(result, "a10736e8-dev01.a10736e8");
    }
}
