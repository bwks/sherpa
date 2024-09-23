use std::fmt;

use clap::ValueEnum;

use serde_derive::{Deserialize, Serialize};

#[derive(PartialEq, Clone, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum DeviceModels {
    AristaVeos,
    CiscoAsav,
    CiscoCsr1000v,
    CiscoCat8000v,
    CiscoCat9000v,
    CiscoIosxrv9000,
    CiscoNexus9300v,
    CiscoIosv,
    CiscoIosvl2,
    #[serde(rename(
        serialize = "juniper_vjunos_router",
        deserialize = "juniper_vjunos_router"
    ))]
    JuniperVjunosRouter,
    #[serde(rename(
        serialize = "juniper_vjunos_switch",
        deserialize = "juniper_vjunos_switch"
    ))]
    JuniperVjunosSwitch,
    NokiaVsr,
    NvidiaCumulus,
    CentosLinux,
    FedoraLinux,
    RedhatLinux,
    OpensuseLinux,
    SuseLinux,
    UbuntuLinux,
}
impl fmt::Display for DeviceModels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceModels::AristaVeos => write!(f, "arista_veos"),
            DeviceModels::CiscoAsav => write!(f, "cisco_asav"),
            DeviceModels::CiscoCsr1000v => write!(f, "cisco_csr1000v"),
            DeviceModels::CiscoCat8000v => write!(f, "cisco_cat8000v"),
            DeviceModels::CiscoCat9000v => write!(f, "cisco_cat9000v"),
            DeviceModels::CiscoIosxrv9000 => write!(f, "cisco_iosxrv9000"),
            DeviceModels::CiscoNexus9300v => write!(f, "cisco_nexus9300v"),
            DeviceModels::CiscoIosv => write!(f, "cisco_iosv"),
            DeviceModels::CiscoIosvl2 => write!(f, "cisco_iosvl2"),
            DeviceModels::JuniperVjunosRouter => write!(f, "juniper_vjunos_router"),
            DeviceModels::JuniperVjunosSwitch => write!(f, "juniper_vjunos_switch"),
            DeviceModels::NokiaVsr => write!(f, "nokia_vsr"),
            DeviceModels::NvidiaCumulus => write!(f, "nvidia_cumulus"),
            DeviceModels::CentosLinux => write!(f, "centos_linux"),
            DeviceModels::FedoraLinux => write!(f, "fedora_linux"),
            DeviceModels::RedhatLinux => write!(f, "rhel_linux"),
            DeviceModels::OpensuseLinux => write!(f, "opensuse_linux"),
            DeviceModels::SuseLinux => write!(f, "suse_linux"),
            DeviceModels::UbuntuLinux => write!(f, "ubuntu_linux"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Manufacturers {
    Cisco,
    Arista,
    Juniper,
    Nvidia,
    Nokia,
    Canonical,
    Redhat,
    Suse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OsVariants {
    Asa,
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
    #[serde(rename(serialize = "pc-i440fx-4.2", deserialize = "pc-i440fx-4.2"))]
    PcI440Fx_4_2,
}
impl fmt::Display for MachineTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MachineTypes::PcQ35_6_2 => write!(f, "pc-q35-6.2"),
            MachineTypes::PcI440Fx_4_2 => write!(f, "pc-i440fx-4.2"),
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

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BiosTypes {
    #[default]
    SeaBios,
    Uefi,
}
impl fmt::Display for BiosTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BiosTypes::SeaBios => write!(f, "sea_bios"),
            BiosTypes::Uefi => write!(f, "uefi"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceModel {
    pub version: String,
    pub name: DeviceModels,
    pub os_variant: OsVariants,
    pub manufacturer: Manufacturers,
    pub bios_type: BiosTypes,
    pub cpu_count: u8,
    pub cpu_architecture: CpuArchitecture,
    pub machine_type: MachineTypes,
    pub memory: u16,
    pub disk_count: u8,
    pub cdrom_iso: Option<String>,
    pub interface_count: u8,
    pub interface_prefix: String,
    pub interface_type: InterfaceTypes,
    pub management_interface: bool,
    pub reserved_interface_count: u8,
}
impl DeviceModel {
    #[allow(dead_code)]
    pub fn get_model(device_model: DeviceModels) -> DeviceModel {
        match device_model {
            DeviceModels::AristaVeos => DeviceModel::arista_veos(),
            DeviceModels::CiscoAsav => DeviceModel::cisco_asav(),
            DeviceModels::CiscoCsr1000v => DeviceModel::cisco_csr1000v(),
            DeviceModels::CiscoCat8000v => DeviceModel::cisco_cat8000v(),
            DeviceModels::CiscoCat9000v => DeviceModel::cisco_cat9000v(),
            DeviceModels::CiscoIosxrv9000 => DeviceModel::cisco_iosxrv9000(),
            DeviceModels::CiscoNexus9300v => DeviceModel::cisco_nexus9300v(),
            DeviceModels::CiscoIosv => DeviceModel::cisco_iosv(),
            DeviceModels::CiscoIosvl2 => DeviceModel::cisco_iosvl2(),
            DeviceModels::JuniperVjunosRouter => DeviceModel::juniper_vjunos_router(),
            DeviceModels::JuniperVjunosSwitch => DeviceModel::juniper_vjunos_switch(),
            DeviceModels::NokiaVsr => DeviceModel::nokia_vsr(),
            DeviceModels::NvidiaCumulus => DeviceModel::nvidia_cumulus(),
            DeviceModels::CentosLinux => DeviceModel::centos_linux(),
            DeviceModels::FedoraLinux => DeviceModel::fedora_linux(),
            DeviceModels::RedhatLinux => DeviceModel::redhat_linux(),
            DeviceModels::OpensuseLinux => DeviceModel::opensuse_linux(),
            DeviceModels::SuseLinux => DeviceModel::suse_linux(),
            DeviceModels::UbuntuLinux => DeviceModel::ubuntu_linux(),
        }
    }
    pub fn arista_veos() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::AristaVeos,
            os_variant: OsVariants::Eos,
            manufacturer: Manufacturers::Arista,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            memory: 4096,
            disk_count: 1,
            cdrom_iso: Some("aboot.iso".to_owned()),
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_asav() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoAsav,
            os_variant: OsVariants::Asa,
            manufacturer: Manufacturers::Cisco,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            memory: 2048,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_csr1000v() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoCsr1000v,
            os_variant: OsVariants::Iosxe,
            manufacturer: Manufacturers::Cisco,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::Vmxnet3,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            memory: 4096,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_cat8000v() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoCat8000v,
            os_variant: OsVariants::Iosxe,
            manufacturer: Manufacturers::Cisco,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig0/0/".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            memory: 16384,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_cat9000v() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoCat9000v,
            os_variant: OsVariants::Iosxe,
            manufacturer: Manufacturers::Cisco,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig0/0/".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            memory: 16384,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_iosxrv9000() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoIosxrv9000,
            os_variant: OsVariants::Iosxr,
            manufacturer: Manufacturers::Cisco,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig0/0/0/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            memory: 16384,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: true,
            reserved_interface_count: 2,
        }
    }
    pub fn cisco_nexus9300v() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoNexus9300v,
            os_variant: OsVariants::Nxos,
            manufacturer: Manufacturers::Cisco,
            bios_type: BiosTypes::Uefi,
            interface_count: 8,
            interface_prefix: "Eth1/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 10240,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_iosv() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoIosv,
            os_variant: OsVariants::Ios,
            manufacturer: Manufacturers::Cisco,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig0/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            memory: 1024,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_iosvl2() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoIosvl2,
            os_variant: OsVariants::Ios,
            manufacturer: Manufacturers::Cisco,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::E1000,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            memory: 1024,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
    pub fn juniper_vjunos_router() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::JuniperVjunosRouter,
            os_variant: OsVariants::Junos,
            manufacturer: Manufacturers::Juniper,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 5120,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: true,
            reserved_interface_count: 2,
        }
    }
    pub fn juniper_vjunos_switch() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::JuniperVjunosSwitch,
            os_variant: OsVariants::Junos,
            manufacturer: Manufacturers::Juniper,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 5120,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: true,
            reserved_interface_count: 2,
        }
    }
    pub fn nvidia_cumulus() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::NvidiaCumulus,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Nvidia,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "swp".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 2048,
            disk_count: 2,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
    pub fn nokia_vsr() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::NokiaVsr,
            os_variant: OsVariants::Sros,
            manufacturer: Manufacturers::Nokia,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::default(),
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            memory: 4096,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn centos_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CentosLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Redhat,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 1024,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
    pub fn fedora_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::FedoraLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Redhat,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 1024,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
    pub fn redhat_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::RedhatLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Redhat,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 1024,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
    pub fn suse_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::SuseLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Suse,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 1024,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
    pub fn opensuse_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::OpensuseLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Suse,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 1024,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
    pub fn ubuntu_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::UbuntuLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Canonical,
            bios_type: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            memory: 1024,
            disk_count: 1,
            cdrom_iso: None,
            management_interface: false,
            reserved_interface_count: 0,
        }
    }
}
