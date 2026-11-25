//! Cloud Hypervisor's REST API, spoken over the VMM's Unix socket.

use std::path::Path;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, UnixConnector, Uri};
use serde_json::{Value, json};
use tracing::instrument;

use super::error::MicroVmError;
use super::{MicroVmNic, MicroVmState};

const API_PREFIX: &str = "/api/v1/";
/// NICs on PCI segment 0, which also holds the host bridge, the disks and the RNG.
const NICS_ON_FIRST_SEGMENT: usize = 24;
/// NICs on each further PCI segment: its 32 slots less the host bridge, with one spare.
const NICS_PER_SEGMENT: usize = 30;

/// Everything `vm.create` needs to define one microVM.
pub(crate) struct VmDefinition<'a> {
    pub(crate) kernel: &'a Path,
    pub(crate) initramfs: &'a Path,
    pub(crate) cmdline: &'a str,
    pub(crate) cpus: u8,
    pub(crate) memory_mib: u64,
    pub(crate) disks: &'a [(&'a Path, bool)],
    pub(crate) nics: &'a [MicroVmNic],
    pub(crate) serial_socket: &'a Path,
}

/// Send `body` to `endpoint` with `method` and return the status and the response body.
async fn call(
    socket: &Path,
    method: Method,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<(StatusCode, Bytes), MicroVmError> {
    let api_error = |message: String| MicroVmError::Api {
        socket: socket.to_path_buf(),
        endpoint: endpoint.to_owned(),
        message,
    };
    let client: Client<UnixConnector, Full<Bytes>> = Client::unix();
    let uri: hyper::Uri = Uri::new(socket, &format!("{API_PREFIX}{endpoint}")).into();
    let payload = match body {
        Some(value) => Bytes::from(value.to_string()),
        None => Bytes::new(),
    };
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .body(Full::new(payload))
        .map_err(|error| api_error(error.to_string()))?;
    let response = client
        .request(request)
        .await
        .map_err(|error| api_error(error.to_string()))?;
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|error| api_error(error.to_string()))?
        .to_bytes();
    Ok((status, bytes))
}

/// Call `endpoint` and fail unless it answers with a success status.
async fn call_ok(
    socket: &Path,
    method: Method,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Bytes, MicroVmError> {
    let (status, bytes) = call(socket, method, endpoint, body).await?;
    if status.is_success() {
        return Ok(bytes);
    }
    Err(MicroVmError::ApiStatus {
        endpoint: endpoint.to_owned(),
        status: status.as_u16(),
        body: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

/// The `vm.create` body for `definition`.
pub(crate) fn vm_config(definition: &VmDefinition<'_>) -> Value {
    let disks: Vec<Value> = definition
        .disks
        .iter()
        .map(|(path, readonly)| json!({ "path": path, "readonly": readonly, "image_type": "Raw" }))
        .collect();
    let net: Vec<Value> = definition
        .nics
        .iter()
        .enumerate()
        .map(|(index, nic)| {
            json!({
                "tap": nic.tap,
                "mac": nic.mac,
                "id": format!("net{index}"),
                "pci_segment": pci_segment(index),
            })
        })
        .collect();
    let segments = definition
        .nics
        .len()
        .checked_sub(1)
        .map_or(1, |last| pci_segment(last) + 1);
    json!({
        "cpus": { "boot_vcpus": definition.cpus, "max_vcpus": definition.cpus },
        "memory": { "size": definition.memory_mib * 1024 * 1024 },
        "payload": {
            "kernel": definition.kernel,
            "initramfs": definition.initramfs,
            "cmdline": definition.cmdline,
        },
        "disks": disks,
        "net": net,
        "serial": { "mode": "Socket", "socket": definition.serial_socket },
        "console": { "mode": "Off" },
        "rng": { "src": "/dev/urandom" },
        "platform": { "num_pci_segments": segments },
    })
}

/// The PCI segment of the NIC at `index`: Cloud Hypervisor has 32 slots per segment.
fn pci_segment(index: usize) -> usize {
    match index.checked_sub(NICS_ON_FIRST_SEGMENT) {
        None => 0,
        Some(beyond) => 1 + beyond / NICS_PER_SEGMENT,
    }
}

/// Whether the VMM behind `socket` answers.
#[instrument(level = "debug")]
pub(crate) async fn ping(socket: &Path) -> bool {
    call_ok(socket, Method::GET, "vmm.ping", None).await.is_ok()
}

/// Define the microVM from `config` and boot it.
#[instrument(skip(config), level = "debug")]
pub(crate) async fn create_and_boot(socket: &Path, config: &Value) -> Result<(), MicroVmError> {
    call_ok(socket, Method::PUT, "vm.create", Some(config)).await?;
    call_ok(socket, Method::PUT, "vm.boot", None).await?;
    Ok(())
}

/// The state of the microVM behind `socket`: stopped when the VMM does not answer.
#[instrument(level = "debug")]
pub(crate) async fn vm_state(socket: &Path) -> MicroVmState {
    let Ok(bytes) = call_ok(socket, Method::GET, "vm.info", None).await else {
        return MicroVmState::Stopped;
    };
    let state = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|info| info.get("state").and_then(Value::as_str).map(str::to_owned));
    match state.as_deref() {
        Some("Running") => MicroVmState::Running,
        Some("Paused") => MicroVmState::Paused,
        _ => MicroVmState::Stopped,
    }
}

/// Add `nic` to the running microVM behind `socket`; Cloud Hypervisor creates its tap.
///
/// # Errors
///
/// Fails when the VMM refuses the device.
#[instrument(level = "debug")]
pub async fn add_nic(socket: &Path, nic: &MicroVmNic, id: &str) -> Result<(), MicroVmError> {
    let body = json!({ "tap": nic.tap, "mac": nic.mac, "id": id });
    call_ok(socket, Method::PUT, "vm.add-net", Some(&body)).await?;
    Ok(())
}

/// Ask the VMM behind `socket` to stop its microVM and exit.
#[instrument(level = "debug")]
pub(crate) async fn shutdown_vmm(socket: &Path) -> Result<(), MicroVmError> {
    call_ok(socket, Method::PUT, "vmm.shutdown", None).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privileged_microvm_001_the_vm_definition_carries_every_disk_nic_and_the_serial_socket() {
        let nics = [
            MicroVmNic {
                tap: "mgmt-tap".to_owned(),
                mac: "02:00:00:00:01:00".to_owned(),
                guest_name: "eth0".to_owned(),
            },
            MicroVmNic {
                tap: "data-tap".to_owned(),
                mac: "02:00:00:00:01:01".to_owned(),
                guest_name: "eth1".to_owned(),
            },
        ];
        let disks = [
            (Path::new("/base.raw"), true),
            (Path::new("/config.iso"), true),
            (Path::new("/node.raw"), false),
        ];
        let config = vm_config(&VmDefinition {
            kernel: Path::new("/vmlinuz"),
            initramfs: Path::new("/initramfs.img"),
            cmdline: "console=ttyS0",
            cpus: 2,
            memory_mib: 4096,
            disks: &disks,
            nics: &nics,
            serial_socket: Path::new("/run/1.serial"),
        });
        assert_eq!(config["memory"]["size"], 4096_u64 * 1024 * 1024);
        assert_eq!(config["disks"][0]["readonly"], true);
        assert_eq!(config["disks"][2]["readonly"], false);
        assert_eq!(config["disks"][1]["image_type"], "Raw");
        assert_eq!(config["net"][1]["tap"], "data-tap");
        assert_eq!(config["net"][1]["id"], "net1");
        assert_eq!(config["serial"]["mode"], "Socket");
        assert_eq!(config["serial"]["socket"], "/run/1.serial");
        assert_eq!(config["platform"]["num_pci_segments"], 1);
    }

    #[test]
    fn privileged_microvm_011_nics_beyond_one_pci_segment_go_on_further_segments() {
        let nics: Vec<MicroVmNic> = (0..60)
            .map(|index| MicroVmNic {
                tap: format!("tap{index}"),
                mac: format!("02:00:00:00:00:{index:02x}"),
                guest_name: format!("eth{index}"),
            })
            .collect();
        let config = vm_config(&VmDefinition {
            kernel: Path::new("/vmlinuz"),
            initramfs: Path::new("/initramfs.img"),
            cmdline: "console=ttyS0",
            cpus: 1,
            memory_mib: 1024,
            disks: &[],
            nics: &nics,
            serial_socket: Path::new("/run/1.serial"),
        });
        let segments = config["platform"]["num_pci_segments"]
            .as_u64()
            .expect("segments");
        let mut per_segment = vec![0; usize::try_from(segments).expect("count")];
        for nic in config["net"].as_array().expect("net") {
            let segment = nic["pci_segment"].as_u64().expect("segment");
            assert!(segment < segments);
            per_segment[usize::try_from(segment).expect("index")] += 1;
        }
        assert!(
            per_segment.iter().all(|count| *count <= 31),
            "{per_segment:?}"
        );
        assert_eq!(per_segment.iter().sum::<usize>(), 60);
    }
}
