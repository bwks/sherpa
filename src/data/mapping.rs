pub use crate::libvirt::DomainTemplate;
// Device name to ip address mapping
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
