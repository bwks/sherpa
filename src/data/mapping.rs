use serde_derive::{Deserialize, Serialize};

pub use crate::libvirt::DomainTemplate;
// Device name to IP address mapping
pub struct DeviceIp {
    pub name: String,
    pub ip_address: String,
}

// Data used to clone disk for VM creation
pub struct CloneDisk {
    pub src: String,
    pub dst: String,
}

pub struct ZtpTemplates {
    pub arista_eos: String,
    pub aruba_aos: String,
    pub cumulus_linux: String,
    pub cisco_iosv: String,
    pub cisco_iosxe: String,
    pub juniper_vjunos: String,
}

pub struct BootServer {
    pub template: DomainTemplate,
    pub copy_disks: Vec<CloneDisk>,
}

/// Interfaces Connection Map
// Each device has a loopback assigned from the 127.127.127.0/24 range
// Connections will be created between devices with UDP tunnels with ports in the 10k range.
// Interfaces with no defined connection will be set to 'down' status
// In the domain XML config, the source is the remote peer.
#[derive(Debug, Deserialize, Serialize)]
pub struct InterfaceConnection {
    pub local_id: u8,
    pub local_port: u16,
    pub local_loopback: String,
    pub source_id: u8,
    pub source_port: u16,
    pub source_loopback: String,
}
