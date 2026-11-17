use serde_derive::{Deserialize, Serialize};

use super::disk::{
    DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskImageCloneMethod, DiskTargets,
};

/// Data used to clone a disk for VM creation.
#[derive(Clone, Debug)]
pub struct CloneDisk {
    pub src: String,
    pub dst: String,
    pub disk_size: Option<u16>,
    pub method: DiskImageCloneMethod,
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

/// A QEMU command-line argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QemuCommand {
    pub param: String,
    pub value: String,
}
impl QemuCommand {
    #[must_use]
    pub fn juniper_router() -> Vec<Self> {
        vec![Self {
            param: "-smbios".to_owned(),
            value: "type=1,product=VM-VMX,family=lab".to_owned(),
        }]
    }
    #[must_use]
    pub fn juniper_switch() -> Vec<Self> {
        vec![Self {
            param: "-smbios".to_owned(),
            value: "type=1,product=VM-VEX".to_owned(),
        }]
    }
    #[must_use]
    pub fn juniper_srxv3() -> Vec<Self> {
        vec![Self {
            param: "-machine".to_owned(),
            value: "smbios-entry-point-type=32".to_owned(),
        }]
    }
    #[must_use]
    pub fn juniper_evolved() -> Vec<Self> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_juniper_router_smbios() {
        let cmds = QemuCommand::juniper_router();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].param, "-smbios");
        assert_eq!(cmds[0].value, "type=1,product=VM-VMX,family=lab");
    }

    #[test]
    fn test_juniper_switch_smbios() {
        let cmds = QemuCommand::juniper_switch();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].param, "-smbios");
        assert_eq!(cmds[0].value, "type=1,product=VM-VEX");
    }

    #[test]
    fn test_juniper_srxv3_machine_type() {
        let cmds = QemuCommand::juniper_srxv3();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].param, "-machine");
        assert_eq!(cmds[0].value, "smbios-entry-point-type=32");
    }

    #[test]
    fn test_juniper_evolved_three_smbios_entries() {
        let cmds = QemuCommand::juniper_evolved();
        assert_eq!(cmds.len(), 3);
        assert!(cmds.iter().all(|c| c.param == "-smbios"));
        // First entry sets vendor
        assert!(cmds[0].value.contains("vendor=Bochs"));
        // Third entry contains the chassis serial
        assert!(cmds[2].value.contains("chassis_no=0"));
    }
}
