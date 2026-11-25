//! The config a microVM's init reads from its config ISO: `node.toml`, `entrypoint.sh` and
//! the files to place in the container root.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use tracing::instrument;

use super::error::MicroVmError;
use super::{MicroVmNic, io_error};

const CONFIG_VOLUME_ID: &str = "SHERPA";
const BUSYBOX: &str = "/.sherpa/busybox";
const DEFAULT_PATH: &str = "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// What the guest init sets up before it runs the container's process.
#[derive(Debug, Clone)]
pub struct GuestConfig {
    pub hostname: String,
    pub mgmt_address: Option<String>,
    pub mgmt_gateway: Option<String>,
    pub nics: Vec<MicroVmNic>,
    pub argv: Vec<String>,
    pub env: Vec<String>,
    pub user: Option<String>,
    pub working_dir: Option<String>,
    pub volumes: Vec<String>,
}

#[derive(Serialize)]
struct NodeToml<'a> {
    hostname: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    mgmt_address: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mgmt_gateway: Option<&'a str>,
    interfaces: BTreeMap<&'a str, &'a str>,
}

/// Quote `word` for a POSIX shell.
fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// Whether `user` is root, as Docker reads a `user` setting.
fn is_root(user: &str) -> bool {
    matches!(user, "" | "0" | "root" | "0:0" | "root:root")
}

/// The `node.toml` the init reads: hostname, management address and the name of each NIC by
/// MAC, management first.
pub(crate) fn node_toml(config: &GuestConfig) -> Result<String, MicroVmError> {
    let node = NodeToml {
        hostname: &config.hostname,
        mgmt_address: config.mgmt_address.as_deref(),
        mgmt_gateway: config.mgmt_gateway.as_deref(),
        interfaces: config
            .nics
            .iter()
            .map(|nic| (nic.mac.as_str(), nic.guest_name.as_str()))
            .collect(),
    };
    toml::to_string(&node).map_err(MicroVmError::GuestConfig)
}

/// The script the init hands PID 1 to: a getty on the serial console, then the container's
/// process with only its own environment, as its user, from its working directory.
pub(crate) fn entrypoint_script(config: &GuestConfig) -> String {
    let mut env = vec![
        DEFAULT_PATH.to_owned(),
        format!("HOSTNAME={}", config.hostname),
    ];
    if config.user.as_deref().is_none_or(is_root) {
        env.push("HOME=/root".to_owned());
    }
    // Docker passes on only `NAME=value` entries; `env` would run any other as the command.
    env.extend(config.env.iter().filter(|var| var.contains('=')).cloned());
    let mut command = vec![BUSYBOX.to_owned(), "env".to_owned(), "-i".to_owned()];
    command.extend(env.iter().map(|var| shell_quote(var)));
    if let Some(user) = config.user.as_deref().filter(|user| !is_root(user)) {
        command.extend([BUSYBOX.to_owned(), "chpst".to_owned(), "-u".to_owned()]);
        command.push(shell_quote(user));
    }
    command.extend(config.argv.iter().map(|arg| shell_quote(arg)));
    let working_dir = config.working_dir.as_deref().unwrap_or("/");
    format!(
        "{BUSYBOX} setsid {BUSYBOX} sh -c 'while :; do {BUSYBOX} getty -L 115200 ttyS0 vt100; \
         sleep 1; done' &\ncd {}\nexec {}\n",
        shell_quote(working_dir),
        command.join(" ")
    )
}

/// The host source and container path of a Docker bind `volume` (`src:dst[:mode]`).
fn split_volume(volume: &str) -> Result<(&str, &str), MicroVmError> {
    let mut parts = volume.splitn(3, ':');
    match (parts.next(), parts.next()) {
        (Some(src), Some(dst)) if !src.is_empty() && dst.starts_with('/') => Ok((src, dst)),
        _ => Err(MicroVmError::Volume {
            volume: volume.to_owned(),
        }),
    }
}

/// Copy `src`, a file or a folder, to `dst`.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), MicroVmError> {
    let io = io_error;
    if src.is_dir() {
        fs::create_dir_all(dst).map_err(io("create", dst))?;
        for entry in fs::read_dir(src).map_err(io("read", src))? {
            let entry = entry.map_err(io("read", src))?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(io("create", parent))?;
    }
    fs::copy(src, dst).map_err(io("copy", src))?;
    Ok(())
}

/// `container_path` under `root`, without leaving it.
fn under(root: &Path, container_path: &str) -> PathBuf {
    let relative: PathBuf = Path::new(container_path)
        .components()
        .filter(|part| matches!(part, Component::Normal(_)))
        .collect();
    root.join(relative)
}

/// Write `config` to the folder `staging` and pack it into the ISO `iso_path`. Each bind
/// volume's source is copied to its container path under `files/`.
///
/// # Errors
///
/// Fails when a volume has no container path, a source cannot be copied, or the ISO cannot be
/// written.
#[instrument(skip(config), fields(hostname = %config.hostname), level = "debug")]
pub fn write_config_iso(
    config: &GuestConfig,
    staging: &Path,
    iso_path: &Path,
) -> Result<(), MicroVmError> {
    let io = io_error;
    if staging.exists() {
        fs::remove_dir_all(staging).map_err(io("remove", staging))?;
    }
    let files = staging.join("files");
    fs::create_dir_all(&files).map_err(io("create", &files))?;
    let node = staging.join("node.toml");
    fs::write(&node, node_toml(config)?).map_err(io("write", &node))?;
    let entrypoint = staging.join("entrypoint.sh");
    fs::write(&entrypoint, entrypoint_script(config)).map_err(io("write", &entrypoint))?;
    for volume in &config.volumes {
        let (src, dst) = split_volume(volume)?;
        copy_tree(Path::new(src), &under(&files, dst))?;
    }
    iso::create_iso(staging, iso_path, CONFIG_VOLUME_ID).map_err(|source| MicroVmError::ConfigIso {
        path: iso_path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GuestConfig {
        GuestConfig {
            hostname: "r1".to_owned(),
            mgmt_address: Some("172.31.0.21/24".to_owned()),
            mgmt_gateway: Some("172.31.0.1".to_owned()),
            nics: vec![
                MicroVmNic {
                    tap: "t0".to_owned(),
                    mac: "02:00:00:00:01:00".to_owned(),
                    guest_name: "eth0".to_owned(),
                },
                MicroVmNic {
                    tap: "t1".to_owned(),
                    mac: "02:00:00:00:01:01".to_owned(),
                    guest_name: "e1-1".to_owned(),
                },
            ],
            argv: vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "echo 'hi'; exec init".to_owned(),
            ],
            env: vec!["CEOS=1".to_owned()],
            user: None,
            working_dir: None,
            volumes: vec![],
        }
    }

    #[test]
    fn privileged_microvm_002_the_entrypoint_runs_the_command_with_only_its_own_environment() {
        let script = entrypoint_script(&config());
        assert!(script.contains("/.sherpa/busybox env -i "));
        assert!(script.contains("'CEOS=1'"));
        assert!(script.contains(r"'sh' '-c' 'echo '\''hi'\''; exec init'"));
        assert!(!script.contains("chpst"));
    }

    #[test]
    fn privileged_microvm_012_an_environment_entry_without_a_value_is_left_out() {
        let mut config = config();
        config.env.push("MAPETH0:1".to_owned());
        let script = entrypoint_script(&config);
        assert!(!script.contains("MAPETH0"));
        assert!(script.contains("'CEOS=1'"));
    }

    #[test]
    fn privileged_microvm_003_a_non_root_user_runs_the_command_through_chpst() {
        let mut config = config();
        config.user = Some("user:user".to_owned());
        let script = entrypoint_script(&config);
        assert!(script.contains("chpst -u 'user:user' 'sh'"));
        assert!(!script.contains("HOME=/root"));
    }

    #[test]
    fn privileged_microvm_004_node_toml_names_each_nic_by_mac() {
        let toml = node_toml(&config()).expect("node.toml");
        assert!(toml.contains(r#"hostname = "r1""#));
        assert!(toml.contains(r#"mgmt_address = "172.31.0.21/24""#));
        assert!(toml.contains(r#""02:00:00:00:01:01" = "e1-1""#));
    }

    #[test]
    fn privileged_microvm_005_a_volume_lands_at_its_container_path_in_the_iso_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("startup-config");
        fs::write(&src, "hostname r1\n").expect("write");
        let mut config = config();
        config.volumes = vec![format!(
            "{}:/mnt/sherpa-ztp/startup-config:ro",
            src.display()
        )];
        let staging = dir.path().join("cfg");
        write_config_iso(&config, &staging, &dir.path().join("config.iso")).expect("iso");
        let copied = staging.join("files/mnt/sherpa-ztp/startup-config");
        assert_eq!(fs::read_to_string(copied).expect("read"), "hostname r1\n");
        assert!(dir.path().join("config.iso").exists());
    }

    #[test]
    fn privileged_microvm_006_a_volume_without_a_container_path_is_refused() {
        assert!(matches!(
            split_volume("/only/source"),
            Err(MicroVmError::Volume { .. })
        ));
    }
}
