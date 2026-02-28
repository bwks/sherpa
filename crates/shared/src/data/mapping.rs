use serde_derive::{Deserialize, Serialize};

use super::disk::{DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskTargets};

#[derive(Clone, Debug)]
// Device name to IP address mapping
pub struct NodeConnection {
    pub name: String,
    pub ip_address: String,
    // pub mac_address: String,
    pub ssh_port: u16,
}

// Data used to clone disk for VM creation
pub struct CloneDisk {
    pub src: String,
    pub dst: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NodeDisk {
    pub disk_device: DiskDevices,
    pub driver_name: DiskDrivers,
    pub driver_format: DiskFormats,
    pub src_file: String,
    pub target_dev: DiskTargets,
    pub target_bus: DiskBuses,
}

/// Interfaces Connection Map
// Each device has a loopback assigned from the 127.127.127.0/24 range
// Connections will be created between devices with UDP tunnels with ports in the 10k range.
// Interfaces with no defined connection will be set to 'down' status
// In the domain XML config, the source is the remote peer.
#[derive(Debug, Deserialize, Serialize)]
pub struct InterfaceConnection {
    pub local_id: u16,
    pub local_port: u16,
    pub local_loopback: String,
    pub source_id: u16,
    pub source_port: u16,
    pub source_loopback: String,
}

#[derive(Clone, Debug)]
// Qemu commamnd-line arguments mapping
pub struct QemuCommand {
    pub param: String,
    pub value: String,
}
impl QemuCommand {
    pub fn juniper_vrouter() -> Vec<Self> {
        vec![Self {
            param: "-smbios".to_owned(),
            value: "type=1,product=VM-VMX,family=lab".to_owned(),
        }]
    }
    pub fn juniper_vswitch() -> Vec<Self> {
        vec![Self {
            param: "-smbios".to_owned(),
            value: "type=1,product=VM-VEX".to_owned(),
        }]
    }
    pub fn juniper_vevolved() -> Vec<Self> {
        vec![
            Self {
            param: "-smbios".to_owned(),
            value: "type=0,vendor=Bochs,version=Bochs".to_owned(),
        },
            Self {
            param: "-smbios".to_owned(),
            value: "type=3,manufacturer=Bochs".to_owned(),
        },
            Self {
            param: "-smbios".to_owned(),
            value: "type=1,manufacturer=Bochs,product=Bochs,serial=chassis_no=0:slot=0:type=1:assembly_id=0x0D20:platform=251:master=0:channelized=no".to_owned(),
        },
        ]
    }
    pub fn ignition_config(path: &str) -> Vec<Self> {
        vec![Self {
            param: "-fw_cfg".to_owned(),
            value: format!("name=opt/org.flatcar-linux/config,file={path}"),
        }]
    }
}
