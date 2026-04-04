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
    pub disk_size: Option<u16>,
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
    pub fn juniper_vsrxv3() -> Vec<Self> {
        vec![Self {
            param: "-machine".to_owned(),
            value: "smbios-entry-point-type=32".to_owned(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_juniper_vrouter_smbios() {
        let cmds = QemuCommand::juniper_vrouter();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].param, "-smbios");
        assert_eq!(cmds[0].value, "type=1,product=VM-VMX,family=lab");
    }

    #[test]
    fn test_juniper_vswitch_smbios() {
        let cmds = QemuCommand::juniper_vswitch();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].param, "-smbios");
        assert_eq!(cmds[0].value, "type=1,product=VM-VEX");
    }

    #[test]
    fn test_juniper_vsrxv3_machine_type() {
        let cmds = QemuCommand::juniper_vsrxv3();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].param, "-machine");
        assert_eq!(cmds[0].value, "smbios-entry-point-type=32");
    }

    #[test]
    fn test_juniper_vevolved_three_smbios_entries() {
        let cmds = QemuCommand::juniper_vevolved();
        assert_eq!(cmds.len(), 3);
        assert!(cmds.iter().all(|c| c.param == "-smbios"));
        // First entry sets vendor
        assert!(cmds[0].value.contains("vendor=Bochs"));
        // Third entry contains the chassis serial
        assert!(cmds[2].value.contains("chassis_no=0"));
    }

    #[test]
    fn test_ignition_config_path_embedded() {
        let path = "/var/lib/ignition.json";
        let cmds = QemuCommand::ignition_config(path);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].param, "-fw_cfg");
        assert_eq!(
            cmds[0].value,
            format!("name=opt/org.flatcar-linux/config,file={path}")
        );
    }
}
