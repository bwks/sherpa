use std::fmt;

use clap::ValueEnum;

use serde_derive::{Deserialize, Serialize};

use crate::core::konst::MTU_JUMBO_INT;
use crate::data::{DiskBuses, MgmtInterfaces};

#[derive(Default, PartialEq, Clone, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum DeviceModels {
    #[default]
    CustomUnknown,
    AristaVeos,
    ArubaAoscx,
    CiscoAsav,
    CiscoCsr1000v,
    CiscoCat8000v,
    CiscoCat9000v,
    CiscoIosxrv9000,
    CiscoNexus9300v,
    CiscoIosv,
    CiscoIosvl2,
    JuniperVrouter,
    JuniperVswitch,
    JuniperVsrx,
    JuniperVsrxv3,
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
            DeviceModels::ArubaAoscx => write!(f, "aruba_aoscx"),
            DeviceModels::CiscoAsav => write!(f, "cisco_asav"),
            DeviceModels::CiscoCsr1000v => write!(f, "cisco_csr1000v"),
            DeviceModels::CiscoCat8000v => write!(f, "cisco_cat8000v"),
            DeviceModels::CiscoCat9000v => write!(f, "cisco_cat9000v"),
            DeviceModels::CiscoIosxrv9000 => write!(f, "cisco_iosxrv9000"),
            DeviceModels::CiscoNexus9300v => write!(f, "cisco_nexus9300v"),
            DeviceModels::CiscoIosv => write!(f, "cisco_iosv"),
            DeviceModels::CiscoIosvl2 => write!(f, "cisco_iosvl2"),
            DeviceModels::JuniperVrouter => write!(f, "juniper_vrouter"),
            DeviceModels::JuniperVswitch => write!(f, "juniper_vswitch"),
            DeviceModels::JuniperVsrx => write!(f, "juniper_vsrx"),
            DeviceModels::JuniperVsrxv3 => write!(f, "juniper_vsrxv3"),
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
            DeviceModels::CustomUnknown => write!(f, "custom_unknown"),
        }
    }
}

impl DeviceModels {
    pub fn needs_ztp_server(&self) -> bool {
        matches!(self, DeviceModels::AristaVeos | DeviceModels::ArubaAoscx)
    }
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Manufacturers {
    Arista,
    Aruba,
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
    Aos,
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
impl fmt::Display for OsVariants {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OsVariants::Asa => write!(f, "asa"),
            OsVariants::Aos => write!(f, "aos"),
            OsVariants::CumulusLinux => write!(f, "cumulus_linux"),
            OsVariants::Eos => write!(f, "eos"),
            OsVariants::Junos => write!(f, "junos"),
            OsVariants::Ios => write!(f, "ios"),
            OsVariants::Iosxe => write!(f, "iosxe"),
            OsVariants::Iosxr => write!(f, "iosxr"),
            OsVariants::Linux => write!(f, "linux"),
            OsVariants::Nxos => write!(f, "nxos"),
            OsVariants::Server2012 => write!(f, "server_2012"),
            OsVariants::Sros => write!(f, "sros"),
            OsVariants::Unknown => write!(f, "unknown"),
        }
    }
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
    #[serde(rename(serialize = "pc", deserialize = "pc"))]
    Pc, // alias of pc-q35-X.Y - Qemu version dependent
    #[serde(rename(serialize = "q35", deserialize = "q35"))]
    Q35, // alias of pc-i440fx-X.Y - Qemu version dependent
    #[serde(rename(serialize = "pc-q35-5.0", deserialize = "pc-q35-5.0"))]
    PcQ35_5_0,
    #[serde(rename(serialize = "pc-q35-5.2", deserialize = "pc-q35-5.2"))]
    PcQ35_5_2,
    #[serde(rename(serialize = "pc-q35-6.2", deserialize = "pc-q35-6.2"))]
    PcQ35_6_2,
    #[serde(rename(serialize = "pc-q35-6.0", deserialize = "pc-q35-6.0"))]
    PcQ35_6_0,
    #[serde(rename(serialize = "pc-q35-8.0", deserialize = "pc-q35-8.0"))]
    PcQ35_8_0,
    #[serde(rename(serialize = "pc-i440fx-5.1", deserialize = "pc-i440fx-5.1"))]
    PcI440Fx_5_1,
    #[serde(rename(serialize = "pc-i440fx-4.2", deserialize = "pc-i440fx-4.2"))]
    PcI440Fx_4_2,
}
impl fmt::Display for MachineTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MachineTypes::Q35 => write!(f, "q35"),
            MachineTypes::Pc => write!(f, "pc"),
            MachineTypes::PcQ35_5_0 => write!(f, "pc-q35-5.0"),
            MachineTypes::PcQ35_5_2 => write!(f, "pc-q35-5.2"),
            MachineTypes::PcQ35_6_0 => write!(f, "pc-q35-6.0"),
            MachineTypes::PcQ35_6_2 => write!(f, "pc-q35-6.2"),
            MachineTypes::PcQ35_8_0 => write!(f, "pc-q35-8.0"),
            MachineTypes::PcI440Fx_5_1 => write!(f, "pc-i440fx-5.1"),
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
    #[serde(rename(serialize = "cloud-init", deserialize = "cloud-init"))]
    CloudInit,
    Cdrom,
    Disk,
    Http,
    Ignition,
    Ipxe,
    Tftp,
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
    pub hdd_bus: DiskBuses,
    pub cdrom: Option<String>,
    pub cdrom_bus: DiskBuses,
    pub ztp_enable: bool,
    pub ztp_method: ZtpMethods,
    pub ztp_username: Option<String>,
    pub ztp_password: Option<String>,
    pub ztp_password_auth: bool,
    pub interface_count: u8,
    pub interface_prefix: String,
    pub interface_type: InterfaceTypes,
    pub interface_mtu: u16,
    pub first_interface_index: u8,
    pub dedicated_management_interface: bool,
    pub management_interface: MgmtInterfaces,
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
            hdd_bus: DiskBuses::default(),
            cdrom: None,
            cdrom_bus: DiskBuses::default(),
            ztp_enable: false,
            ztp_method: ZtpMethods::None,
            ztp_username: None,
            ztp_password: None,
            ztp_password_auth: false,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::default(),
            interface_mtu: 1500,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::default(),
            reserved_interface_count: 0,
        }
    }
}

impl DeviceModel {
    #[allow(dead_code)]
    pub fn get_model(device_model: DeviceModels) -> DeviceModel {
        match device_model {
            DeviceModels::AristaVeos => DeviceModel::arista_veos(),
            DeviceModels::ArubaAoscx => DeviceModel::aruba_aoscx(),
            DeviceModels::CiscoAsav => DeviceModel::cisco_asav(),
            DeviceModels::CiscoCsr1000v => DeviceModel::cisco_csr1000v(),
            DeviceModels::CiscoCat8000v => DeviceModel::cisco_cat8000v(),
            DeviceModels::CiscoCat9000v => DeviceModel::cisco_cat9000v(),
            DeviceModels::CiscoIosxrv9000 => DeviceModel::cisco_iosxrv9000(),
            DeviceModels::CiscoNexus9300v => DeviceModel::cisco_nexus9300v(),
            DeviceModels::CiscoIosv => DeviceModel::cisco_iosv(),
            DeviceModels::CiscoIosvl2 => DeviceModel::cisco_iosvl2(),
            DeviceModels::JuniperVrouter => DeviceModel::juniper_vrouter(),
            DeviceModels::JuniperVswitch => DeviceModel::juniper_vswitch(),
            DeviceModels::JuniperVsrx => DeviceModel::juniper_vsrx(),
            DeviceModels::JuniperVsrxv3 => DeviceModel::juniper_vsrxv3(),
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
            DeviceModels::CustomUnknown => DeviceModel::default(),
        }
    }
    pub fn arista_veos() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::AristaVeos,
            os_variant: OsVariants::Eos,
            manufacturer: Manufacturers::Arista,
            bios: BiosTypes::SeaBios,
            interface_count: 24,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Pc,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Sata,
            cdrom: Some("aboot.iso".to_owned()),
            cdrom_bus: DiskBuses::Ide,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Usb,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Management1,
            reserved_interface_count: 0,
        }
    }
    pub fn aruba_aoscx() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::ArubaAoscx,
            os_variant: OsVariants::Aos,
            manufacturer: Manufacturers::Arista,
            bios: BiosTypes::SeaBios,
            interface_count: 24,
            interface_prefix: "1/1/".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Usb,
            ztp_password_auth: true,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Mgmt,
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
            interface_prefix: "GigabitEthernet0".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Pc,
            vmx_enabled: false,
            memory: 2048,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Ide,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Management0_0,
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
            interface_count: 16,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::Vmxnet3,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: true,
            first_interface_index: 1,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::GigabitEthernet1,
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
            interface_count: 16,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 16384,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::GigabitEthernet1,
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
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 18432,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::GigabitEthernet0_0,
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
            interface_count: 16,
            interface_prefix: "Gig0/0/0/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 16384,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::MgmtEth0Rp0Cpu0_0,
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
            interface_count: 64,
            interface_prefix: "Eth1/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Pc,
            vmx_enabled: false,
            memory: 10240,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Ide,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Mgmt0,
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
            interface_count: 16,
            interface_prefix: "Gig0/".to_owned(),
            interface_type: InterfaceTypes::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Pc,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Virtio,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Disk,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::GigabitEthernet0_0,
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
            interface_count: 16, // Crashes if more than 16 interfaces are defined
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceTypes::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Pc,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Virtio,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Disk,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::GigabitEthernet0_0,
            reserved_interface_count: 0,
        }
    }
    pub fn juniper_vrouter() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::JuniperVrouter,
            os_variant: OsVariants::Junos,
            manufacturer: Manufacturers::Juniper,
            bios: BiosTypes::SeaBios,
            interface_count: 16,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Pc,
            vmx_enabled: true,
            memory: 5120,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Virtio,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Fxp0,
            reserved_interface_count: 0,
        }
    }
    pub fn juniper_vswitch() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::JuniperVswitch,
            os_variant: OsVariants::Junos,
            manufacturer: Manufacturers::Juniper,
            bios: BiosTypes::SeaBios,
            interface_count: 24,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: true,
            memory: 5120,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Fxp0,
            reserved_interface_count: 2,
        }
    }
    pub fn juniper_vsrx() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::JuniperVsrx,
            os_variant: OsVariants::Junos,
            manufacturer: Manufacturers::Juniper,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: true,
            memory: 4096,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Usb,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Fxp0,
            reserved_interface_count: 0,
        }
    }
    pub fn juniper_vsrxv3() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::JuniperVsrxv3,
            os_variant: OsVariants::Junos,
            manufacturer: Manufacturers::Juniper,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: true,
            memory: 4096,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Fxp0,
            reserved_interface_count: 0,
        }
    }
    pub fn cumulus_linux() -> DeviceModel {
        DeviceModel {
            version: "latest".to_owned(),
            name: DeviceModels::CumulusLinux,
            os_variant: OsVariants::CumulusLinux,
            manufacturer: Manufacturers::Nvidia,
            bios: BiosTypes::SeaBios,
            interface_count: 24,
            interface_prefix: "swp".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 2048,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Usb,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Eth0,
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
            interface_count: 16,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceTypes::default(),
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Pc,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Ide,
            ztp_enable: false,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::default(),
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Eth0,
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
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
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
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
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
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
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
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
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
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
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
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
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
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceTypes::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            machine_type: MachineTypes::Q35,
            vmx_enabled: false,
            memory: 2048,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethods::Ignition,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
}
