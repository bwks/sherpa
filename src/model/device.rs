use serde_derive::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceModels {
    AristaVeos,
    CiscoCsr1000v,
    CiscoCat8000v,
    CiscoCat9000v,
    CiscoIosxrv9000,
    CiscoNexus9300v,
    CiscoIosv,
    CiscoIosvl2,
    NokiaSros,
    NvidiaCumulus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Manufacturers {
    Cisco,
    Arista,
    Juniper,
    Nvidia,
    Nokia,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OsVariants {
    Ios,
    Iosxe,
    Iosxr,
    Nxos,
    Eos,
    Junos,
    Linux,
    Sros,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CpuArchitecture {
    #[default]
    X86_64,
}
impl fmt::Display for CpuArchitecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CpuArchitecture::X86_64 => write!(f, "x86_64"),
        }
    }
}

#[allow(non_camel_case_types)]
#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub enum MachineTypes {
    #[default]
    #[serde(rename(serialize = "pc-q35-6.2", deserialize = "pc-q35-6.2"))]
    // kvm value: pc-q35-6.2
    PcQ35_6_2,
}
impl fmt::Display for MachineTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MachineTypes::PcQ35_6_2 => write!(f, "pc-q35-6.2"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InterfaceTypes {
    E1000,
    #[default]
    Virtio,
    Vmxnet3,
}
impl fmt::Display for InterfaceTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InterfaceTypes::E1000 => write!(f, "e1000"),
            InterfaceTypes::Virtio => write!(f, "virtio"),
            InterfaceTypes::Vmxnet3 => write!(f, "vmxnet3"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceModel {
    pub name: DeviceModels,
    pub os_variant: OsVariants,
    pub manufacturer: Manufacturers,
    pub interface_count: u8,
    pub interface_prefix: String,
    pub interface_type: InterfaceTypes,
    pub cpu_count: u8,
    pub cpu_architecture: CpuArchitecture,
    pub machine_type: MachineTypes,
    pub memory: u16,
    pub disk_count: u8,
    pub version: String,
}
impl DeviceModel {
    pub fn get_model(device_model: DeviceModels) -> DeviceModel {
        match device_model {
            DeviceModels::AristaVeos => DeviceModel::arista_veos(),
            DeviceModels::CiscoCsr1000v => DeviceModel::cisco_csr1000v(),
            DeviceModels::CiscoCat8000v => DeviceModel::cisco_cat8000v(),
            DeviceModels::CiscoCat9000v => DeviceModel::cisco_cat9000v(),
            DeviceModels::CiscoIosxrv9000 => DeviceModel::cisco_iosxrv9000(),
            DeviceModels::CiscoNexus9300v => DeviceModel::cisco_nexus9300v(),
            DeviceModels::CiscoIosv => DeviceModel::cisco_iosv(),
            DeviceModels::CiscoIosvl2 => DeviceModel::cisco_iosvl2(),
            DeviceModels::NokiaSros => DeviceModel::nokia_sros(),
            DeviceModels::NvidiaCumulus => DeviceModel::nvidia_cumulus(),
        }
    }
    pub fn arista_veos() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::AristaVeos,
            os_variant: OsVariants::Eos,
            manufacturer: Manufacturers::Arista,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 2048,
            disk_count: 1,
            version: "latest".to_owned(),
        }
    }
    pub fn cisco_csr1000v() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoCsr1000v,
            os_variant: OsVariants::Iosxe,
            manufacturer: Manufacturers::Cisco,
            interface_count: 8,
            interface_prefix: "Gig0/".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 4096,
            disk_count: 1,
            version: "latest".to_owned(),
        }
    }
    pub fn cisco_cat8000v() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoCat8000v,
            os_variant: OsVariants::Iosxe,
            manufacturer: Manufacturers::Cisco,
            interface_count: 8,
            interface_prefix: "Gig0/0/".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 16384,
            disk_count: 1,
            version: "latest".to_owned(),
        }
    }
    pub fn cisco_cat9000v() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoCat9000v,
            os_variant: OsVariants::Iosxe,
            manufacturer: Manufacturers::Cisco,
            interface_count: 8,
            interface_prefix: "Gig0/0/".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 16384,
            disk_count: 1,
            version: "latest".to_owned(),
        }
    }
    pub fn cisco_iosxrv9000() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoIosxrv9000,
            os_variant: OsVariants::Iosxr,
            manufacturer: Manufacturers::Cisco,
            interface_count: 8,
            interface_prefix: "Gig0/0/0/".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 16384,
            disk_count: 1,
            version: "latest".to_owned(),
        }
    }
    pub fn cisco_nexus9300v() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoNexus9300v,
            os_variant: OsVariants::Nxos,
            manufacturer: Manufacturers::Cisco,
            interface_count: 8,
            interface_prefix: "Int0/".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 8096,
            disk_count: 1,
            version: "latest".to_owned(),
        }
    }
    pub fn cisco_iosv() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoIosv,
            os_variant: OsVariants::Ios,
            manufacturer: Manufacturers::Cisco,
            interface_count: 8,
            interface_prefix: "Gig0/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 1024,
            disk_count: 1,
            version: "vios-adventerprisek9-m.SPA.159-3.M6".to_owned(),
        }
    }
    pub fn cisco_iosvl2() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoIosvl2,
            os_variant: OsVariants::Ios,
            manufacturer: Manufacturers::Cisco,
            interface_count: 8,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 1024,
            disk_count: 1,
            version: "latest".to_owned(),
        }
    }
    pub fn nvidia_cumulus() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::NvidiaCumulus,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Nvidia,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 2048,
            disk_count: 1,
            version: "latest".to_owned(),
        }
    }
    pub fn nokia_sros() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::NokiaSros,
            os_variant: OsVariants::Sros,
            manufacturer: Manufacturers::Nokia,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 2048,
            disk_count: 1,
            version: "latest".to_owned(),
        }
    }
}
