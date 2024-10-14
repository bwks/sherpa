use std::fmt;

use clap::ValueEnum;

use serde_derive::{Deserialize, Serialize};

use crate::core::konst::MTU_JUMBO_INT;

#[derive(Default, PartialEq, Clone, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum DeviceModels {
    #[default]
    UnknownUnknown,
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
    CumulusLinux,
    CentosLinux,
    FedoraLinux,
    RedhatLinux,
    OpensuseLinux,
    SuseLinux,
    UbuntuLinux,
    FlatcarLinux,
    WindowsServer2012,
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
            DeviceModels::CumulusLinux => write!(f, "cumulus_linux"),
            DeviceModels::CentosLinux => write!(f, "centos_linux"),
            DeviceModels::FedoraLinux => write!(f, "fedora_linux"),
            DeviceModels::RedhatLinux => write!(f, "rhel_linux"),
            DeviceModels::OpensuseLinux => write!(f, "opensuse_linux"),
            DeviceModels::SuseLinux => write!(f, "suse_linux"),
            DeviceModels::UbuntuLinux => write!(f, "ubuntu_linux"),
            DeviceModels::WindowsServer2012 => write!(f, "windows_server"),
            DeviceModels::FlatcarLinux => write!(f, "flatcar_linux"),
            DeviceModels::UnknownUnknown => write!(f, "unknown_unknown"),
        }
    }
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Manufacturers {
    Arista,
    Canonical,
    Cisco,
    Juniper,
    Kinvolk,
    Microsoft,
    Nokia,
    Nvidia,
    Redhat,
    Suse,
    #[default]
    Unknown,
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OsVariants {
    Asa,
    CumulusLinux,
    Eos,
    Junos,
    Ios,
    Iosxe,
    Iosxr,
    Linux,
    Nxos,
    Server2012,
    Sros,
    #[default]
    Unknown,
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

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ZtpMethods {
    #[default]
    Cdrom,
    Http,
    Tftp,
    Ipxe,
    Usb,
    None,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceModel {
    pub version: String,
    pub name: DeviceModels,
    pub os_variant: OsVariants,
    pub manufacturer: Manufacturers,
    pub bios: BiosTypes,
    pub cpu_count: u8,
    pub cpu_architecture: CpuArchitecture,
    pub machine_type: MachineTypes,
    pub vmx_enabled: bool,
    pub memory: u16,
    pub hdd_count: u8,
    pub cdrom: Option<String>,
    pub ztp_enable: bool,
    pub ztp_method: ZtpMethods,
    pub ztp_username: Option<String>,
    pub ztp_password: Option<String>,
    pub ztp_password_auth: bool,
    pub interface_count: u8,
    pub interface_prefix: String,
    pub interface_type: InterfaceTypes,
    pub interface_mtu: u16,
    pub management_interface: bool,
    pub reserved_interface_count: u8,
}

impl Default for DeviceModel {
    fn default() -> Self {
        Self {
            version: "0.0.0".to_owned(),
            name: DeviceModels::default(),
            os_variant: OsVariants::default(),
            manufacturer: Manufacturers::default(),
            bios: BiosTypes::default(),
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::default(),
            machine_type: MachineTypes::default(),
            vmx_enabled: false,
            memory: 1024,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_method: ZtpMethods::default(),
            ztp_username: None,
            ztp_password: None,
            ztp_password_auth: false,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::default(),
            interface_mtu: 1500,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
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
            DeviceModels::CumulusLinux => DeviceModel::cumulus_linux(),
            DeviceModels::CentosLinux => DeviceModel::centos_linux(),
            DeviceModels::FedoraLinux => DeviceModel::fedora_linux(),
            DeviceModels::RedhatLinux => DeviceModel::redhat_linux(),
            DeviceModels::OpensuseLinux => DeviceModel::opensuse_linux(),
            DeviceModels::SuseLinux => DeviceModel::suse_linux(),
            DeviceModels::UbuntuLinux => DeviceModel::ubuntu_linux(),
            DeviceModels::FlatcarLinux => DeviceModel::flatcar_linux(),
            DeviceModels::WindowsServer2012 => todo!(),
            DeviceModels::UnknownUnknown => DeviceModel::default(),
        }
    }
    pub fn arista_veos() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::AristaVeos,
            os_variant: OsVariants::Eos,
            manufacturer: Manufacturers::Arista,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            vmx_enabled: false,
            memory: 4096,
            hdd_count: 1,
            cdrom: Some("aboot.iso".to_owned()),
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Http,
            ztp_password_auth: false,
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
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 2048,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: false,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Http,
            ztp_password_auth: false,
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
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::Vmxnet3,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 4096,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: true,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_cat8000v() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoCat8000v,
            os_variant: OsVariants::Iosxe,
            manufacturer: Manufacturers::Cisco,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 16384,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
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
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig0/0/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 18432,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
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
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig0/0/0/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 16384,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: false,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Http,
            ztp_password_auth: false,
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
            bios: BiosTypes::Uefi,
            interface_count: 8,
            interface_prefix: "Eth1/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 10240,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: false,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Http,
            ztp_password_auth: false,
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
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig0/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 1024,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: false,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::None,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_iosvl2() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CiscoIosvl2,
            os_variant: OsVariants::Ios,
            manufacturer: Manufacturers::Cisco,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 1024,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: false,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::None,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn juniper_vjunos_router() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::JuniperVjunosRouter,
            os_variant: OsVariants::Junos,
            manufacturer: Manufacturers::Juniper,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 5120,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: false,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Http,
            ztp_password_auth: false,
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
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 5120,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: false,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Http,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 2,
        }
    }
    pub fn cumulus_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CumulusLinux,
            os_variant: OsVariants::CumulusLinux,
            manufacturer: Manufacturers::Nvidia,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "swp".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 2048,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Usb,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn nokia_vsr() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::NokiaVsr,
            os_variant: OsVariants::Sros,
            manufacturer: Manufacturers::Nokia,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::default(),
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcI440Fx_4_2,
            vmx_enabled: false,
            memory: 4096,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: false,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::default(),
            ztp_password_auth: false,
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
            bios: BiosTypes::SeaBios,
            interface_count: 0,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 1024,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn fedora_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::FedoraLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Redhat,
            bios: BiosTypes::SeaBios,
            interface_count: 0,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 1024,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn redhat_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::RedhatLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Redhat,
            bios: BiosTypes::SeaBios,
            interface_count: 0,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 1024,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn suse_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::SuseLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Suse,
            bios: BiosTypes::SeaBios,
            interface_count: 0,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 1024,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn opensuse_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::OpensuseLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Suse,
            bios: BiosTypes::SeaBios,
            interface_count: 0,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 1024,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn ubuntu_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::UbuntuLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Canonical,
            bios: BiosTypes::SeaBios,
            interface_count: 0,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 1024,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
    pub fn flatcar_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::FlatcarLinux,
            os_variant: OsVariants::Linux,
            manufacturer: Manufacturers::Microsoft,
            bios: BiosTypes::SeaBios,
            interface_count: 0,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::PcQ35_6_2,
            vmx_enabled: false,
            memory: 2048,
            hdd_count: 1,
            cdrom: None,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            management_interface: true,
            reserved_interface_count: 0,
        }
    }
}
