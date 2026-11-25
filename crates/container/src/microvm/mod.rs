//! Container images run inside Cloud Hypervisor microVMs, so that a model that needs
//! `--privileged` is privileged in its own guest kernel and never on the host. See
//! `docs/architecture/MICROVM.md`.

mod api;
mod error;
mod guest;
mod image;

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tokio::process::Command;
use tokio::time::{Instant, sleep};
use tracing::instrument;

pub use api::add_nic;
pub use error::MicroVmError;
pub use guest::{GuestConfig, write_config_iso};
pub use image::ensure_base_disk;

pub const MICROVM_DIR: &str = "/opt/sherpa/microvm";
pub const MICROVM_RUN_DIR: &str = "/opt/sherpa/run/microvm";
const HYPERVISOR_BINARY: &str = "cloud-hypervisor";
const KERNEL_FILE: &str = "vmlinuz";
const INITRAMFS_FILE: &str = "initramfs.img";
const API_SOCKET: &str = "api.sock";
const PID_FILE: &str = "ch.pid";
const LOG_FILE: &str = "ch.log";
const KERNEL_CMDLINE: &str = "console=ttyS0 net.ifnames=0 panic=-1 quiet";
const API_WAIT: Duration = Duration::from_secs(10);
const STOP_WAIT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(200);

/// One NIC of a microVM: the host tap Cloud Hypervisor creates, its MAC, and its name in the
/// guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicroVmNic {
    pub tap: String,
    pub mac: String,
    pub guest_name: String,
}

/// What a microVM reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicroVmState {
    Running,
    Paused,
    Stopped,
}

/// A microVM to start: its disks, its NICs, its serial socket and its size.
#[derive(Debug, Clone)]
pub struct MicroVmSpec {
    pub name: String,
    pub runtime_dir: PathBuf,
    pub base_disk: PathBuf,
    pub config_iso: PathBuf,
    pub node_disk: PathBuf,
    pub node_disk_bytes: u64,
    pub serial_socket: PathBuf,
    pub cpus: u8,
    pub memory_mib: u64,
    pub nics: Vec<MicroVmNic>,
}

/// The runtime folder of node `node_name` in lab `lab_id`.
#[must_use]
pub fn runtime_dir(lab_id: &str, node_name: &str) -> PathBuf {
    Path::new(MICROVM_RUN_DIR).join(lab_id).join(node_name)
}

/// The path of `file` in the microVM folder.
fn microvm_file(file: &str) -> PathBuf {
    Path::new(MICROVM_DIR).join(file)
}

/// An I/O error on `path` while doing `action`.
fn io_error(action: &'static str, path: &Path) -> impl FnOnce(io::Error) -> MicroVmError {
    let path = path.to_path_buf();
    move |source| MicroVmError::Io {
        action,
        path,
        source,
    }
}

/// Start Cloud Hypervisor for `spec`, define the microVM through its API and boot it. Cloud
/// Hypervisor creates every NIC's tap, which goes when it exits. A microVM that does not boot
/// is stopped again, so a retry starts clean.
///
/// # Errors
///
/// Fails when a folder or disk cannot be made, Cloud Hypervisor cannot start, or the API
/// refuses the microVM.
#[instrument(skip(spec), fields(name = %spec.name))]
pub async fn start_microvm(spec: &MicroVmSpec) -> Result<(), MicroVmError> {
    let dir = &spec.runtime_dir;
    fs::create_dir_all(dir).map_err(io_error("create", dir))?;
    if let Some(parent) = spec.serial_socket.parent() {
        fs::create_dir_all(parent).map_err(io_error("create", parent))?;
    }
    let _ = fs::remove_file(&spec.serial_socket);
    let socket = dir.join(API_SOCKET);
    let _ = fs::remove_file(&socket);
    let disk = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&spec.node_disk)
        .map_err(io_error("create", &spec.node_disk))?;
    disk.set_len(spec.node_disk_bytes)
        .map_err(io_error("size", &spec.node_disk))?;

    let log_path = dir.join(LOG_FILE);
    let log = File::create(&log_path).map_err(io_error("create", &log_path))?;
    let log_err = log.try_clone().map_err(io_error("open", &log_path))?;
    let mut child = Command::new(microvm_file(HYPERVISOR_BINARY))
        .arg("--api-socket")
        .arg(format!("path={}", socket.display()))
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err)
        .process_group(0)
        .spawn()
        .map_err(|source| MicroVmError::Spawn {
            name: spec.name.clone(),
            source,
        })?;
    if let Some(pid) = child.id() {
        let pid_path = dir.join(PID_FILE);
        fs::write(&pid_path, pid.to_string()).map_err(io_error("write", &pid_path))?;
    }
    // Reap the process whenever it exits; the microVM is found again by its socket.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    let deadline = Instant::now() + API_WAIT;
    while !api::ping(&socket).await {
        if Instant::now() > deadline {
            let _ = stop_microvm(&spec.name, dir).await;
            return Err(MicroVmError::ApiNotReady {
                name: spec.name.clone(),
                socket,
            });
        }
        sleep(POLL).await;
    }

    let kernel = microvm_file(KERNEL_FILE);
    let initramfs = microvm_file(INITRAMFS_FILE);
    let disks = [
        (spec.base_disk.as_path(), true),
        (spec.config_iso.as_path(), true),
        (spec.node_disk.as_path(), false),
    ];
    let config = api::vm_config(&api::VmDefinition {
        kernel: &kernel,
        initramfs: &initramfs,
        cmdline: KERNEL_CMDLINE,
        cpus: spec.cpus,
        memory_mib: spec.memory_mib,
        disks: &disks,
        nics: &spec.nics,
        serial_socket: &spec.serial_socket,
    });
    if let Err(error) = api::create_and_boot(&socket, &config).await {
        let _ = stop_microvm(&spec.name, dir).await;
        return Err(error);
    }
    tracing::info!(name = %spec.name, nics = spec.nics.len(), "MicroVM booted");
    Ok(())
}

/// The state of the microVM whose runtime folder is `dir`.
#[instrument(level = "debug")]
pub async fn microvm_state(dir: &Path) -> MicroVmState {
    api::vm_state(&dir.join(API_SOCKET)).await
}

/// The Cloud Hypervisor process recorded in `dir`, when it still runs from that folder.
fn recorded_pid(dir: &Path) -> Option<Pid> {
    let pid: i32 = fs::read_to_string(dir.join(PID_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let cmdline = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let socket = dir.join(API_SOCKET);
    let socket = socket.to_string_lossy();
    String::from_utf8_lossy(&cmdline)
        .contains(socket.as_ref())
        .then_some(Pid::from_raw(pid))
}

/// Stop the microVM `name` whose runtime folder is `dir`: ask its VMM to shut down, kill it
/// when it does not, and remove the folder. Its taps go with the process. Stopping a microVM
/// that is not there succeeds.
///
/// # Errors
///
/// Fails when the process cannot be killed or the folder cannot be removed.
#[instrument(level = "debug")]
pub async fn stop_microvm(name: &str, dir: &Path) -> Result<(), MicroVmError> {
    if !dir.exists() {
        return Ok(());
    }
    let socket = dir.join(API_SOCKET);
    if let Err(error) = api::shutdown_vmm(&socket).await {
        tracing::debug!(%name, %error, "VMM did not take the shutdown request");
    }
    let deadline = Instant::now() + STOP_WAIT;
    while let Some(pid) = recorded_pid(dir) {
        if Instant::now() > deadline {
            kill(pid, Signal::SIGKILL).map_err(|source| MicroVmError::Kill {
                name: name.to_owned(),
                pid: pid.as_raw(),
                source,
            })?;
            sleep(POLL).await;
            if recorded_pid(dir).is_some() {
                return Err(MicroVmError::NotStopped {
                    name: name.to_owned(),
                    pid: pid.as_raw(),
                });
            }
            break;
        }
        sleep(POLL).await;
    }
    fs::remove_dir_all(dir).map_err(io_error("remove", dir))?;
    tracing::info!(%name, "MicroVM stopped");
    Ok(())
}

/// Every microVM in the runtime folder, keyed `<node>-<lab>` as its container would be, with
/// its state.
#[instrument(level = "debug")]
pub async fn list_microvms() -> HashMap<String, MicroVmState> {
    let mut states = HashMap::new();
    let Ok(labs) = fs::read_dir(MICROVM_RUN_DIR) else {
        return states;
    };
    for lab in labs.flatten() {
        let lab_id = lab.file_name().to_string_lossy().into_owned();
        let Ok(nodes) = fs::read_dir(lab.path()) else {
            continue;
        };
        for node in nodes.flatten() {
            let node_name = node.file_name().to_string_lossy().into_owned();
            let state = microvm_state(&node.path()).await;
            states.insert(format!("{node_name}-{lab_id}"), state);
        }
    }
    states
}

/// Stop every microVM of lab `lab_id` and remove its runtime folder.
///
/// # Errors
///
/// Fails when one cannot be stopped; the others are still stopped.
#[instrument]
pub async fn stop_lab_microvms(lab_id: &str) -> Result<(), MicroVmError> {
    let lab_dir = Path::new(MICROVM_RUN_DIR).join(lab_id);
    let Ok(nodes) = fs::read_dir(&lab_dir) else {
        return Ok(());
    };
    let mut first_error = None;
    for node in nodes.flatten() {
        let name = format!("{}-{lab_id}", node.file_name().to_string_lossy());
        if let Err(error) = stop_microvm(&name, &node.path()).await {
            tracing::error!(%name, %error, "Cannot stop microVM");
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    fs::remove_dir_all(&lab_dir).map_err(io_error("remove", &lab_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker_connection;

    #[tokio::test]
    #[ignore = "requires root (taps), Docker with alpine:latest, and /opt/sherpa/microvm installed"]
    async fn privileged_microvm_013_a_stopped_microvm_leaves_no_process_tap_or_runtime_folder() {
        let docker = docker_connection().expect("docker");
        let base_disk = ensure_base_disk(&docker, "alpine:latest")
            .await
            .expect("base disk");
        let dir = tempfile::tempdir().expect("tempdir");
        let tap = "smv013-test";
        let nics = vec![MicroVmNic {
            tap: tap.to_owned(),
            mac: "02:53:00:00:13:00".to_owned(),
            guest_name: "eth0".to_owned(),
        }];
        let config = GuestConfig {
            hostname: "mv013".to_owned(),
            mgmt_address: None,
            mgmt_gateway: None,
            nics: nics.clone(),
            argv: vec!["sleep".to_owned(), "3600".to_owned()],
            env: vec![],
            user: None,
            working_dir: None,
            volumes: vec![],
        };
        let config_iso = dir.path().join("config.iso");
        write_config_iso(&config, &dir.path().join("config"), &config_iso).expect("iso");
        let runtime_dir = dir.path().join("run");
        let spec = MicroVmSpec {
            name: "mv013".to_owned(),
            runtime_dir: runtime_dir.clone(),
            base_disk,
            config_iso,
            node_disk: dir.path().join("node.raw"),
            node_disk_bytes: 256 * 1024 * 1024,
            serial_socket: dir.path().join("serial.sock"),
            cpus: 1,
            memory_mib: 256,
            nics,
        };
        start_microvm(&spec).await.expect("start");
        assert_eq!(microvm_state(&runtime_dir).await, MicroVmState::Running);
        let pid = recorded_pid(&runtime_dir).expect("pid");
        assert!(Path::new("/sys/class/net").join(tap).exists());

        stop_microvm("mv013", &runtime_dir).await.expect("stop");
        assert!(!runtime_dir.exists());
        assert!(!Path::new("/sys/class/net").join(tap).exists());
        let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        assert!(cmdline.is_empty(), "Cloud Hypervisor {pid} still runs");
    }
}
