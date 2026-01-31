use std::fmt;

use clap::ValueEnum;

use serde_derive::{Deserialize, Serialize};
use surrealdb::RecordId;

use strum::IntoEnumIterator;
use strum_macros::EnumIter;

use super::cpu::CpuModels;
use super::disk::DiskBuses;
use super::interface::MgmtInterfaces;
use konst::{MTU_JUMBO_INT, MTU_JUMBO_NET, MTU_STD};

#[derive(Default, PartialEq, Clone, Debug, Deserialize, Serialize, ValueEnum, EnumIter)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum NodeModel {
    #[default]
    // Arista
    AristaVeos,
    AristaCeos,

    // Aruba
    ArubaAoscx,

    // Cisco
    CiscoAsav,
    CiscoCsr1000v,
    CiscoCat8000v,
    CiscoCat9000v,
    CiscoIosxrv9000,
    CiscoNexus9300v,
    CiscoIosv,
    CiscoIosvl2,
    CiscoIse,
    CiscoFtdv,

    // Juniper
    JuniperVrouter,
    JuniperVswitch,
    JuniperVevolved,
    JuniperVsrxv3,

    // Nokia
    NokiaSrlinux,

    // Linux
    AlmaLinux,
    RockyLinux,
    AlpineLinux,
    CumulusLinux,
    CentosLinux,
    FedoraLinux,
    RedhatLinux,
    OpensuseLinux,
    SuseLinux,
    UbuntuLinux,
    FlatcarLinux,
    SonicLinux,

    // Windows
    WindowsServer,

    // BSD
    FreeBsd,
    OpenBsd,

    // SQL
    SurrealDb,
    MysqlDb,
    PostgresqlDb,

    // Generic
    GenericContainer,
    GenericUnikernel,
    GenericVm,
}
impl fmt::Display for NodeModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Arista
            NodeModel::AristaVeos => write!(f, "arista_veos"),
            NodeModel::AristaCeos => write!(f, "arista_ceos"),

            // Aruba
            NodeModel::ArubaAoscx => write!(f, "aruba_aoscx"),

            // Cisco
            NodeModel::CiscoAsav => write!(f, "cisco_asav"),
            NodeModel::CiscoCsr1000v => write!(f, "cisco_csr1000v"),
            NodeModel::CiscoCat8000v => write!(f, "cisco_cat8000v"),
            NodeModel::CiscoCat9000v => write!(f, "cisco_cat9000v"),
            NodeModel::CiscoIosxrv9000 => write!(f, "cisco_iosxrv9000"),
            NodeModel::CiscoNexus9300v => write!(f, "cisco_nexus9300v"),
            NodeModel::CiscoIosv => write!(f, "cisco_iosv"),
            NodeModel::CiscoIosvl2 => write!(f, "cisco_iosvl2"),
            NodeModel::CiscoIse => write!(f, "cisco_ise"),
            NodeModel::CiscoFtdv => write!(f, "cisco_ftdv"),

            // Juniper
            NodeModel::JuniperVrouter => write!(f, "juniper_vrouter"),
            NodeModel::JuniperVswitch => write!(f, "juniper_vswitch"),
            NodeModel::JuniperVevolved => write!(f, "juniper_vevolved"),
            NodeModel::JuniperVsrxv3 => write!(f, "juniper_vsrxv3"),

            // Nokia
            NodeModel::NokiaSrlinux => write!(f, "nokia_srlinux"),

            // Linux
            NodeModel::AlmaLinux => write!(f, "alma_linux"),
            NodeModel::RockyLinux => write!(f, "rocky_linux"),
            NodeModel::AlpineLinux => write!(f, "alpine_linux"),
            NodeModel::CumulusLinux => write!(f, "cumulus_linux"),
            NodeModel::CentosLinux => write!(f, "centos_linux"),
            NodeModel::FedoraLinux => write!(f, "fedora_linux"),
            NodeModel::RedhatLinux => write!(f, "redhat_linux"),
            NodeModel::OpensuseLinux => write!(f, "opensuse_linux"),
            NodeModel::SuseLinux => write!(f, "suse_linux"),
            NodeModel::UbuntuLinux => write!(f, "ubuntu_linux"),
            NodeModel::FlatcarLinux => write!(f, "flatcar_linux"),
            NodeModel::SonicLinux => write!(f, "sonic_linux"),

            // Windows
            NodeModel::WindowsServer => write!(f, "windows_server"),

            // SQL
            NodeModel::SurrealDb => write!(f, "surreal_db"),
            NodeModel::MysqlDb => write!(f, "mysql_db"),
            NodeModel::PostgresqlDb => write!(f, "postgresql_db"),

            // BSD
            NodeModel::FreeBsd => write!(f, "free_bsd"),
            NodeModel::OpenBsd => write!(f, "open_bsd"),

            // Generic
            NodeModel::GenericContainer => write!(f, "generic_container"),
            NodeModel::GenericUnikernel => write!(f, "generic_unikernel"),
            NodeModel::GenericVm => write!(f, "generic_vm"),
        }
    }
}
impl NodeModel {
    pub fn to_vec() -> Vec<NodeModel> {
        NodeModel::iter().collect()
    }
}

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, EnumIter)]
#[serde(rename_all = "snake_case")]
pub enum OsVariant {
    // Arista
    Eos,

    // Aruba
    Aos,

    // Cisco
    Asa,
    Ios,
    Iosxe,
    Iosxr,
    Ise,
    Nxos,
    Fxos,

    // Juniper
    Junos,

    // BSD
    Bsd,

    // Linux
    Linux, // Generic
    Nvue,  // Cumlus
    Sonic, // Sonic Linux

    // Windows
    Server2012,

    // Nokia
    Srlinux,

    #[default]
    Unknown,
}
impl fmt::Display for OsVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OsVariant::Asa => write!(f, "asa"),
            OsVariant::Aos => write!(f, "aos"),
            OsVariant::Eos => write!(f, "eos"),
            OsVariant::Junos => write!(f, "junos"),
            OsVariant::Ios => write!(f, "ios"),
            OsVariant::Iosxe => write!(f, "iosxe"),
            OsVariant::Iosxr => write!(f, "iosxr"),
            OsVariant::Ise => write!(f, "ise"),
            OsVariant::Fxos => write!(f, "fxos"),
            OsVariant::Linux => write!(f, "linux"),
            OsVariant::Nvue => write!(f, "nvue"),
            OsVariant::Nxos => write!(f, "nxos"),
            OsVariant::Sonic => write!(f, "sonic"),
            OsVariant::Bsd => write!(f, "bsd"),
            OsVariant::Server2012 => write!(f, "server2012"),
            OsVariant::Srlinux => write!(f, "srlinux"),
            OsVariant::Unknown => write!(f, "unknown"),
        }
    }
}
impl OsVariant {
    pub fn to_vec() -> Vec<OsVariant> {
        OsVariant::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, EnumIter)]
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
impl CpuArchitecture {
    pub fn to_vec() -> Vec<CpuArchitecture> {
        CpuArchitecture::iter().collect()
    }
}

#[allow(non_camel_case_types)]
#[derive(Clone, Debug, Deserialize, Default, Serialize, EnumIter)]
pub enum MachineType {
    #[default]
    #[serde(rename(serialize = "pc", deserialize = "pc"))]
    Pc, // alias of pc-i440fx-X.Y - Qemu version dependent
    #[serde(rename(serialize = "q35", deserialize = "q35"))]
    Q35, // alias of pc-q35-X.Y - Qemu version dependent
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
    #[serde(rename(serialize = "pc-q35-8.1", deserialize = "pc-q35-8.1"))]
    PcQ35_8_1,
    #[serde(rename(serialize = "pc-q35-8.2", deserialize = "pc-q35-8.2"))]
    PcQ35_8_2,
    #[serde(rename(serialize = "pc-i440fx-4.2", deserialize = "pc-i440fx-4.2"))]
    PcI440Fx_4_2,
    #[serde(rename(serialize = "pc-i440fx-5.1", deserialize = "pc-i440fx-5.1"))]
    PcI440Fx_5_1,
    #[serde(rename(serialize = "pc-i440fx-8.0", deserialize = "pc-i440fx-8.0"))]
    PcI440Fx_8_0,
    #[serde(rename(serialize = "pc-i440fx-8.1", deserialize = "pc-i440fx-8.1"))]
    PcI440Fx_8_1,
    #[serde(rename(serialize = "pc-i440fx-8.2", deserialize = "pc-i440fx-8.2"))]
    PcI440Fx_8_2,
}
impl fmt::Display for MachineType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MachineType::Q35 => write!(f, "q35"),
            MachineType::Pc => write!(f, "pc"),
            MachineType::PcQ35_5_0 => write!(f, "pc-q35-5.0"),
            MachineType::PcQ35_5_2 => write!(f, "pc-q35-5.2"),
            MachineType::PcQ35_6_0 => write!(f, "pc-q35-6.0"),
            MachineType::PcQ35_6_2 => write!(f, "pc-q35-6.2"),
            MachineType::PcQ35_8_0 => write!(f, "pc-q35-8.0"),
            MachineType::PcQ35_8_1 => write!(f, "pc-q35-8.1"),
            MachineType::PcQ35_8_2 => write!(f, "pc-q35-8.2"),
            MachineType::PcI440Fx_5_1 => write!(f, "pc-i440fx-5.1"),
            MachineType::PcI440Fx_4_2 => write!(f, "pc-i440fx-4.2"),
            MachineType::PcI440Fx_8_0 => write!(f, "pc-i440fx-8.0"),
            MachineType::PcI440Fx_8_1 => write!(f, "pc-i440fx-8.1"),
            MachineType::PcI440Fx_8_2 => write!(f, "pc-i440fx-8.2"),
        }
    }
}
impl MachineType {
    pub fn to_vec() -> Vec<MachineType> {
        MachineType::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, EnumIter)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceType {
    // VM
    E1000,
    #[default]
    Virtio,
    Vmxnet3,

    // Container
    Host,
    MacVlan,
}
impl fmt::Display for InterfaceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // VM
            InterfaceType::E1000 => write!(f, "e1000"),
            InterfaceType::Virtio => write!(f, "virtio"),
            InterfaceType::Vmxnet3 => write!(f, "vmxnet3"),

            // Container
            InterfaceType::Host => write!(f, "host"),
            InterfaceType::MacVlan => write!(f, "mac_vlan"),
        }
    }
}
impl InterfaceType {
    pub fn to_vec() -> Vec<InterfaceType> {
        InterfaceType::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, EnumIter)]
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
impl BiosTypes {
    pub fn to_vec() -> Vec<BiosTypes> {
        BiosTypes::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq, EnumIter)]
#[serde(rename_all = "lowercase")]
pub enum ZtpMethod {
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
    Volume,
    None,
}
impl fmt::Display for ZtpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZtpMethod::CloudInit => write!(f, "cloud-init"),
            ZtpMethod::Cdrom => write!(f, "cdrom"),
            ZtpMethod::Disk => write!(f, "disk"),
            ZtpMethod::Http => write!(f, "http"),
            ZtpMethod::Ignition => write!(f, "ignition"),
            ZtpMethod::Ipxe => write!(f, "ipxe"),
            ZtpMethod::Tftp => write!(f, "tftp"),
            ZtpMethod::Usb => write!(f, "usb"),
            ZtpMethod::Volume => write!(f, "volume"),
            ZtpMethod::None => write!(f, "none"),
        }
    }
}
impl ZtpMethod {
    pub fn to_vec() -> Vec<ZtpMethod> {
        ZtpMethod::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq, EnumIter)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    #[default]
    VirtualMachine,
    Container,
    Unikernel,
}
impl fmt::Display for NodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeKind::VirtualMachine => write!(f, "virtual_machine"),
            NodeKind::Container => write!(f, "container"),
            NodeKind::Unikernel => write!(f, "unikernel"),
        }
    }
}
impl NodeKind {
    pub fn to_vec() -> Vec<NodeKind> {
        NodeKind::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NodeConfig {
    pub id: Option<RecordId>,
    pub model: NodeModel,
    pub version: String,
    pub repo: Option<String>,
    pub os_variant: OsVariant,
    pub kind: NodeKind,
    pub bios: BiosTypes,
    pub cpu_count: u8,
    pub cpu_architecture: CpuArchitecture,
    pub cpu_model: CpuModels,
    pub machine_type: MachineType,
    pub vmx_enabled: bool,
    pub memory: u16,
    pub hdd_bus: DiskBuses,
    pub cdrom: Option<String>,
    pub cdrom_bus: DiskBuses,
    pub ztp_enable: bool,
    pub ztp_method: ZtpMethod,
    pub ztp_username: Option<String>,
    pub ztp_password: Option<String>,
    pub ztp_password_auth: bool,
    pub interface_count: u8,
    pub interface_prefix: String,
    pub interface_type: InterfaceType,
    pub interface_mtu: u16,
    pub first_interface_index: u8,
    pub dedicated_management_interface: bool,
    pub management_interface: MgmtInterfaces,
    pub reserved_interface_count: u8,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            id: None,
            model: NodeModel::default(),
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::default(),
            kind: NodeKind::default(),
            bios: BiosTypes::default(),
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::default(),
            cpu_model: CpuModels::default(),
            machine_type: MachineType::default(),
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::default(),
            cdrom: None,
            cdrom_bus: DiskBuses::default(),
            ztp_enable: false,
            ztp_method: ZtpMethod::None,
            ztp_username: None,
            ztp_password: None,
            ztp_password_auth: false,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::default(),
            interface_mtu: 1500,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::default(),
            reserved_interface_count: 0,
        }
    }
}

impl NodeConfig {
    #[allow(dead_code)]
    pub fn get_model(device_model: NodeModel) -> NodeConfig {
        match device_model {
            // Arista
            NodeModel::AristaVeos => NodeConfig::arista_veos(),
            NodeModel::AristaCeos => NodeConfig::arista_ceos(),

            // Aruba
            NodeModel::ArubaAoscx => NodeConfig::aruba_aoscx(),

            // Cisco
            NodeModel::CiscoAsav => NodeConfig::cisco_asav(),
            NodeModel::CiscoCsr1000v => NodeConfig::cisco_csr1000v(),
            NodeModel::CiscoCat8000v => NodeConfig::cisco_cat8000v(),
            NodeModel::CiscoCat9000v => NodeConfig::cisco_cat9000v(),
            NodeModel::CiscoIosxrv9000 => NodeConfig::cisco_iosxrv9000(),
            NodeModel::CiscoNexus9300v => NodeConfig::cisco_nexus9300v(),
            NodeModel::CiscoIosv => NodeConfig::cisco_iosv(),
            NodeModel::CiscoIosvl2 => NodeConfig::cisco_iosvl2(),
            NodeModel::CiscoIse => NodeConfig::cisco_ise(),
            NodeModel::CiscoFtdv => NodeConfig::cisco_ftdv(),

            // Juniper
            NodeModel::JuniperVrouter => NodeConfig::juniper_vrouter(),
            NodeModel::JuniperVswitch => NodeConfig::juniper_vswitch(),
            NodeModel::JuniperVevolved => NodeConfig::juniper_vevolved(),
            NodeModel::JuniperVsrxv3 => NodeConfig::juniper_vsrxv3(),

            // Nokia
            NodeModel::NokiaSrlinux => NodeConfig::nokia_srlinux(),

            // Linux
            NodeModel::AlmaLinux => NodeConfig::alma_linux(),
            NodeModel::RockyLinux => NodeConfig::rocky_linux(),
            NodeModel::AlpineLinux => NodeConfig::alpine_linux(),
            NodeModel::CumulusLinux => NodeConfig::cumulus_linux(),
            NodeModel::CentosLinux => NodeConfig::centos_linux(),
            NodeModel::FedoraLinux => NodeConfig::fedora_linux(),
            NodeModel::RedhatLinux => NodeConfig::redhat_linux(),
            NodeModel::OpensuseLinux => NodeConfig::opensuse_linux(),
            NodeModel::SuseLinux => NodeConfig::suse_linux(),
            NodeModel::UbuntuLinux => NodeConfig::ubuntu_linux(),
            NodeModel::SonicLinux => NodeConfig::sonic_linux(),
            NodeModel::FlatcarLinux => NodeConfig::flatcar_linux(),

            // BSD
            NodeModel::FreeBsd => NodeConfig::free_bsd(),
            NodeModel::OpenBsd => NodeConfig::open_bsd(),

            // Windows
            NodeModel::WindowsServer => NodeConfig::windows_server(),

            // SQL
            NodeModel::MysqlDb => NodeConfig::mysql_db(),
            NodeModel::PostgresqlDb => NodeConfig::postgresql_db(),
            NodeModel::SurrealDb => NodeConfig::surreal_db(),

            // Generic
            NodeModel::GenericContainer => NodeConfig::generic_container(),
            NodeModel::GenericUnikernel => NodeConfig::generic_unikernel(),
            NodeModel::GenericVm => NodeConfig::generic_vm(),
        }
    }
    pub fn arista_veos() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::AristaVeos,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Eos,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 52,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_STD,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Pc,
            vmx_enabled: false,
            memory: 2048,
            hdd_bus: DiskBuses::Sata,
            cdrom: Some("aboot.iso".to_owned()),
            cdrom_bus: DiskBuses::Ide,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Tftp,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Management1,
            reserved_interface_count: 0,
        }
    }
    pub fn arista_ceos() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::AristaCeos,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Eos,
            kind: NodeKind::Container,
            bios: BiosTypes::SeaBios,
            interface_count: 52,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_STD,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::None,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn aruba_aoscx() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::ArubaAoscx,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Aos,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 52,
            interface_prefix: "1/1/".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Tftp,
            ztp_password_auth: true,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Mgmt,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_asav() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoAsav,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Asa,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "GigabitEthernet0".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Pc,
            vmx_enabled: false,
            memory: 2048,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Ide,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Management0_0,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_csr1000v() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoCsr1000v,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Iosxe,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 16,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceType::Vmxnet3,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 3072,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: true,
            first_interface_index: 1,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::GigabitEthernet1,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_cat8000v() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoCat8000v,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Iosxe,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 16,
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::GigabitEthernet1,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_cat9000v() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoCat9000v,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Iosxe,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "Gig0/0/".to_owned(),
            interface_type: InterfaceType::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 18432,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::GigabitEthernet0_0,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_iosxrv9000() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoIosxrv9000,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Iosxr,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::Uefi,
            interface_count: 31,
            interface_prefix: "Gig0/0/0/".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Pc,
            vmx_enabled: false,
            memory: 20480,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::MgmtEth0Rp0Cpu0_0,
            reserved_interface_count: 2,
        }
    }
    pub fn cisco_nexus9300v() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoNexus9300v,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Nxos,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::Uefi,
            interface_count: 64,
            interface_prefix: "Eth1/".to_owned(),
            interface_type: InterfaceType::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Pc,
            vmx_enabled: false,
            memory: 12288,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Ide,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Mgmt0,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_iosv() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoIosv,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Ios,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 16,
            interface_prefix: "Gig0/".to_owned(),
            interface_type: InterfaceType::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Pc,
            vmx_enabled: false,
            memory: 768,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Virtio,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Disk,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::GigabitEthernet0_0,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_iosvl2() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoIosvl2,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Ios,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 16, // Crashes if more than 16 interfaces are defined
            interface_prefix: "Gig".to_owned(),
            interface_type: InterfaceType::E1000,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Pc,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Virtio,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Disk,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::GigabitEthernet0_0,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_ise() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoIse,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Ise,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Pc,
            vmx_enabled: false,
            memory: 16384,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn cisco_ftdv() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoFtdv,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Fxos,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "gig0/".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Pc,
            vmx_enabled: false,
            memory: 8192,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Management0_0,
            reserved_interface_count: 1,
        }
    }
    pub fn juniper_vrouter() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JuniperVrouter,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Junos,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 10,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_NET,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::IvyBridge,
            machine_type: MachineType::Pc,
            vmx_enabled: true,
            memory: 5120,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Fxp0,
            reserved_interface_count: 0,
        }
    }
    pub fn juniper_vswitch() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JuniperVswitch,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Junos,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 10,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_NET,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::IvyBridge,
            machine_type: MachineType::Pc,
            vmx_enabled: true,
            memory: 5120,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Fxp0,
            reserved_interface_count: 0,
        }
    }
    pub fn juniper_vevolved() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JuniperVevolved,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Junos,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::Uefi,
            interface_count: 12,
            interface_prefix: "et-0/0/".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_NET,
            cpu_count: 4,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::IvyBridge,
            machine_type: MachineType::Pc,
            vmx_enabled: true,
            memory: 8192,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Usb,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Re0Mgmt0,
            reserved_interface_count: 0,
        }
    }
    pub fn juniper_vsrxv3() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JuniperVsrxv3,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Junos,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 8,
            interface_prefix: "ge-0/0/".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_NET,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::SandyBridge,
            machine_type: MachineType::PcI440Fx_8_0,
            vmx_enabled: true,
            memory: 4096,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Cdrom,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Fxp0,
            reserved_interface_count: 0,
        }
    }
    pub fn alma_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::AlmaLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn rocky_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::RockyLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn alpine_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::AlpineLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 2,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn cumulus_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CumulusLinux,
            version: "latest".to_owned(),
            repo: None,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 52,
            interface_prefix: "swp".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            os_variant: OsVariant::Nvue,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 2048,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Usb,
            ztp_password_auth: false,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn nokia_srlinux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::NokiaSrlinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Srlinux,
            kind: NodeKind::Container,
            bios: BiosTypes::SeaBios,
            interface_count: 16,
            interface_prefix: "Eth".to_owned(),
            interface_type: InterfaceType::default(),
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::None,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn centos_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CentosLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn fedora_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::FedoraLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn redhat_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::RedhatLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn suse_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::SuseLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn opensuse_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::OpensuseLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Sata,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn ubuntu_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::UbuntuLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn sonic_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::SonicLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 52,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Virtio,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Http,
            ztp_password_auth: true,
            first_interface_index: 1,
            dedicated_management_interface: true,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn flatcar_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::FlatcarLinux,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 2048,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::Ignition,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn free_bsd() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::FreeBsd,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Bsd,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn open_bsd() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::OpenBsd,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Bsd,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 1,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 1024,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn windows_server() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::WindowsServer,
            version: "latest".to_owned(),
            repo: None,
            os_variant: OsVariant::Server2012,
            kind: NodeKind::VirtualMachine,
            bios: BiosTypes::SeaBios,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            interface_mtu: MTU_JUMBO_INT,
            cpu_count: 2,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Q35,
            vmx_enabled: false,
            memory: 4096,
            hdd_bus: DiskBuses::Virtio,
            cdrom: None,
            cdrom_bus: DiskBuses::Sata,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
        }
    }
    pub fn surreal_db() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::SurrealDb,
            repo: None,
            version: "latest".to_owned(),
            kind: NodeKind::Container,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::MacVlan,
            cpu_count: 1,
            memory: 1024,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::None,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
            ..Default::default()
        }
    }
    pub fn mysql_db() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::MysqlDb,
            repo: None,
            version: "latest".to_owned(),
            kind: NodeKind::Container,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::MacVlan,
            cpu_count: 1,
            memory: 1024,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::None,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
            ..Default::default()
        }
    }
    pub fn postgresql_db() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::PostgresqlDb,
            repo: None,
            version: "latest".to_owned(),
            kind: NodeKind::Container,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::MacVlan,
            cpu_count: 1,
            memory: 1024,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::None,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
            ..Default::default()
        }
    }
    pub fn generic_container() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::GenericContainer,
            repo: None,
            version: "latest".to_owned(),
            kind: NodeKind::Container,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::MacVlan,
            cpu_count: 1,
            memory: 1024,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::None,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
            ..Default::default()
        }
    }
    pub fn generic_unikernel() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::GenericUnikernel,
            repo: None,
            version: "latest".to_owned(),
            kind: NodeKind::Unikernel,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            cpu_count: 1,
            memory: 1024,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
            ..Default::default()
        }
    }
    pub fn generic_vm() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::GenericVm,
            repo: None,
            version: "latest".to_owned(),
            kind: NodeKind::VirtualMachine,
            interface_count: 1,
            interface_prefix: "eth".to_owned(),
            interface_type: InterfaceType::Virtio,
            cpu_count: 1,
            memory: 1024,
            ztp_enable: true,
            ztp_username: None,
            ztp_password: None,
            ztp_method: ZtpMethod::CloudInit,
            ztp_password_auth: false,
            first_interface_index: 0,
            dedicated_management_interface: false,
            management_interface: MgmtInterfaces::Eth0,
            reserved_interface_count: 0,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_variants_are_unique() {
        // Ensure no duplicates in the all_variants list
        use std::collections::HashSet;
        let variants = NodeModel::to_vec();
        let unique: HashSet<String> = variants.iter().map(|v| v.to_string()).collect();
        assert_eq!(
            variants.len(),
            unique.len(),
            "all_variants contains duplicates"
        );
    }
}
