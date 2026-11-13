use std::fmt;

use clap::ValueEnum;
use schemars::JsonSchema;
use serde_derive::{Deserialize, Serialize};

use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter};

use super::cpu::CpuModels;
use super::disk::DiskBuses;
use super::interface::MgmtInterfaces;
use crate::konst::{
    CONTAINER_ARISTA_CEOS_REPO, CONTAINER_FORGEJO_REPO, CONTAINER_FRR_REPO,
    CONTAINER_GITLAB_CE_REPO, CONTAINER_HASHICORP_VAULT_REPO, CONTAINER_JUNIPER_CEVOLVED_REPO,
    CONTAINER_KEYCLOAK_REPO, CONTAINER_MONGO_DB_REPO, CONTAINER_NATS_REPO,
    CONTAINER_NOKIA_SRLINUX_REPO, CONTAINER_SURREAL_DB_REPO, MTU_JUMBO_INT, MTU_JUMBO_NET, MTU_STD,
    SSH_PORT, TALOS_API_PORT, TALOS_DEFAULT_BOOT_DISK_SIZE,
};

use super::RecordId;

#[derive(
    Default,
    PartialEq,
    Eq,
    Hash,
    Clone,
    Copy,
    Debug,
    Deserialize,
    Serialize,
    ValueEnum,
    EnumIter,
    JsonSchema,
)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
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
    JuniperCevolved,
    JuniperVsrxv3,

    // Nokia
    NokiaSrlinux,

    // Palo Alto
    PaloaltoPanos,

    // Mikrotik
    MikrotikChr,

    // FRR
    FrrLinux,

    // Linux
    AlmaLinux,
    RockyLinux,
    AlpineLinux,
    CumulusLinux,
    CentosLinux,
    DevboxLinux,
    FedoraLinux,
    RedhatLinux,
    OpensuseLinux,
    SuseLinux,
    UbuntuLinux,
    OmarchyLinux,
    NixosLinux,
    KaliLinux,
    FlatcarLinux,
    SonicLinux,
    TalosLinux,

    // Windows
    DevboxWindows,
    WindowsServer,

    // BSD
    FreeBsd,
    OpenBsd,

    // Application
    ForgejoForge,
    GitlabCe,
    InfrahubServer,
    JenkinsServer,
    NautobotServer,
    NetboxServer,
    SignozServer,
    VirtServer,

    // Messaging
    NatsBus,

    // SQL
    SurrealDb,
    MysqlDb,
    MongoDb,
    PostgresqlDb,

    // HashiCorp
    HashicorpVault,

    // Identity
    KeycloakServer,

    // Generic
    GenericContainer,
    GenericUnikernel,
    GenericVm,

    // Unikernels
    UnikraftUnikernel,
    NanosUnikernel,
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
            NodeModel::JuniperCevolved => write!(f, "juniper_cevolved"),
            NodeModel::JuniperVsrxv3 => write!(f, "juniper_vsrxv3"),

            // Nokia
            NodeModel::NokiaSrlinux => write!(f, "nokia_srlinux"),

            // Palo Alto
            NodeModel::PaloaltoPanos => write!(f, "paloalto_panos"),

            // Mikrotik
            NodeModel::MikrotikChr => write!(f, "mikrotik_chr"),

            // FRR
            NodeModel::FrrLinux => write!(f, "frr_linux"),

            // Linux
            NodeModel::AlmaLinux => write!(f, "alma_linux"),
            NodeModel::RockyLinux => write!(f, "rocky_linux"),
            NodeModel::AlpineLinux => write!(f, "alpine_linux"),
            NodeModel::CumulusLinux => write!(f, "cumulus_linux"),
            NodeModel::CentosLinux => write!(f, "centos_linux"),
            NodeModel::DevboxLinux => write!(f, "devbox_linux"),
            NodeModel::FedoraLinux => write!(f, "fedora_linux"),
            NodeModel::RedhatLinux => write!(f, "redhat_linux"),
            NodeModel::OpensuseLinux => write!(f, "opensuse_linux"),
            NodeModel::SuseLinux => write!(f, "suse_linux"),
            NodeModel::UbuntuLinux => write!(f, "ubuntu_linux"),
            NodeModel::OmarchyLinux => write!(f, "omarchy_linux"),
            NodeModel::NixosLinux => write!(f, "nixos_linux"),
            NodeModel::KaliLinux => write!(f, "kali_linux"),
            NodeModel::FlatcarLinux => write!(f, "flatcar_linux"),
            NodeModel::TalosLinux => write!(f, "talos_linux"),
            NodeModel::SonicLinux => write!(f, "sonic_linux"),

            // Windows
            NodeModel::DevboxWindows => write!(f, "devbox_windows"),
            NodeModel::WindowsServer => write!(f, "windows_server"),

            // Application
            NodeModel::ForgejoForge => write!(f, "forgejo_forge"),
            NodeModel::GitlabCe => write!(f, "gitlab_ce"),
            NodeModel::InfrahubServer => write!(f, "infrahub_server"),
            NodeModel::JenkinsServer => write!(f, "jenkins_server"),
            NodeModel::NautobotServer => write!(f, "nautobot_server"),
            NodeModel::NetboxServer => write!(f, "netbox_server"),
            NodeModel::SignozServer => write!(f, "signoz_server"),
            NodeModel::VirtServer => write!(f, "virt_server"),

            // Messaging
            NodeModel::NatsBus => write!(f, "nats_bus"),

            // SQL
            NodeModel::SurrealDb => write!(f, "surreal_db"),
            NodeModel::MysqlDb => write!(f, "mysql_db"),
            NodeModel::MongoDb => write!(f, "mongo_db"),
            NodeModel::PostgresqlDb => write!(f, "postgresql_db"),

            // HashiCorp
            NodeModel::HashicorpVault => write!(f, "hashicorp_vault"),

            // Identity
            NodeModel::KeycloakServer => write!(f, "keycloak_server"),

            // BSD
            NodeModel::FreeBsd => write!(f, "free_bsd"),
            NodeModel::OpenBsd => write!(f, "open_bsd"),

            // Generic
            NodeModel::GenericContainer => write!(f, "generic_container"),
            NodeModel::GenericUnikernel => write!(f, "generic_unikernel"),
            NodeModel::GenericVm => write!(f, "generic_vm"),

            // Unikernels
            NodeModel::UnikraftUnikernel => write!(f, "unikraft_unikernel"),
            NodeModel::NanosUnikernel => write!(f, "nanos_unikernel"),
        }
    }
}
impl std::str::FromStr for NodeModel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            // Arista
            "arista_veos" => Ok(NodeModel::AristaVeos),
            "arista_ceos" => Ok(NodeModel::AristaCeos),

            // Aruba
            "aruba_aoscx" => Ok(NodeModel::ArubaAoscx),

            // Cisco
            "cisco_asav" => Ok(NodeModel::CiscoAsav),
            "cisco_csr1000v" => Ok(NodeModel::CiscoCsr1000v),
            "cisco_cat8000v" => Ok(NodeModel::CiscoCat8000v),
            "cisco_cat9000v" => Ok(NodeModel::CiscoCat9000v),
            "cisco_iosxrv9000" => Ok(NodeModel::CiscoIosxrv9000),
            "cisco_nexus9300v" => Ok(NodeModel::CiscoNexus9300v),
            "cisco_iosv" => Ok(NodeModel::CiscoIosv),
            "cisco_iosvl2" => Ok(NodeModel::CiscoIosvl2),
            "cisco_ise" => Ok(NodeModel::CiscoIse),
            "cisco_ftdv" => Ok(NodeModel::CiscoFtdv),

            // Juniper
            "juniper_vrouter" => Ok(NodeModel::JuniperVrouter),
            "juniper_vswitch" => Ok(NodeModel::JuniperVswitch),
            "juniper_vevolved" => Ok(NodeModel::JuniperVevolved),
            "juniper_cevolved" => Ok(NodeModel::JuniperCevolved),
            "juniper_vsrxv3" => Ok(NodeModel::JuniperVsrxv3),

            // Nokia
            "nokia_srlinux" => Ok(NodeModel::NokiaSrlinux),

            // Palo Alto
            "paloalto_panos" => Ok(NodeModel::PaloaltoPanos),

            // Mikrotik
            "mikrotik_chr" => Ok(NodeModel::MikrotikChr),

            // FRR
            "frr_linux" => Ok(NodeModel::FrrLinux),

            // Linux
            "alma_linux" => Ok(NodeModel::AlmaLinux),
            "rocky_linux" => Ok(NodeModel::RockyLinux),
            "alpine_linux" => Ok(NodeModel::AlpineLinux),
            "cumulus_linux" => Ok(NodeModel::CumulusLinux),
            "centos_linux" => Ok(NodeModel::CentosLinux),
            "fedora_linux" => Ok(NodeModel::FedoraLinux),
            "redhat_linux" => Ok(NodeModel::RedhatLinux),
            "opensuse_linux" => Ok(NodeModel::OpensuseLinux),
            "suse_linux" => Ok(NodeModel::SuseLinux),
            "ubuntu_linux" => Ok(NodeModel::UbuntuLinux),
            "omarchy_linux" => Ok(NodeModel::OmarchyLinux),
            "nixos_linux" => Ok(NodeModel::NixosLinux),
            "kali_linux" => Ok(NodeModel::KaliLinux),
            "flatcar_linux" => Ok(NodeModel::FlatcarLinux),
            "talos_linux" => Ok(NodeModel::TalosLinux),
            "sonic_linux" => Ok(NodeModel::SonicLinux),

            // Devboxes
            "devbox_linux" => Ok(NodeModel::DevboxLinux),
            "devbox_windows" => Ok(NodeModel::DevboxWindows),

            // Windows
            "windows_server" => Ok(NodeModel::WindowsServer),

            // Application
            "forgejo_forge" => Ok(NodeModel::ForgejoForge),
            "gitlab_ce" => Ok(NodeModel::GitlabCe),
            "infrahub_server" => Ok(NodeModel::InfrahubServer),
            "jenkins_server" => Ok(NodeModel::JenkinsServer),
            "nautobot_server" => Ok(NodeModel::NautobotServer),
            "netbox_server" => Ok(NodeModel::NetboxServer),
            "signoz_server" => Ok(NodeModel::SignozServer),
            "virt_server" => Ok(NodeModel::VirtServer),

            // Messaging
            "nats_bus" => Ok(NodeModel::NatsBus),

            // SQL
            "surreal_db" => Ok(NodeModel::SurrealDb),
            "mysql_db" => Ok(NodeModel::MysqlDb),
            "mongo_db" => Ok(NodeModel::MongoDb),
            "postgresql_db" => Ok(NodeModel::PostgresqlDb),

            // HashiCorp
            "hashicorp_vault" => Ok(NodeModel::HashicorpVault),

            // Identity
            "keycloak_server" => Ok(NodeModel::KeycloakServer),

            // BSD
            "free_bsd" => Ok(NodeModel::FreeBsd),
            "open_bsd" => Ok(NodeModel::OpenBsd),

            // Generic
            "generic_container" => Ok(NodeModel::GenericContainer),
            "generic_unikernel" => Ok(NodeModel::GenericUnikernel),
            "generic_vm" => Ok(NodeModel::GenericVm),

            // Unikernels
            "unikraft_unikernel" => Ok(NodeModel::UnikraftUnikernel),
            "nanos_unikernel" => Ok(NodeModel::NanosUnikernel),

            _ => Err(format!("Unknown node model: {s}")),
        }
    }
}
impl NodeModel {
    #[must_use]
    pub fn to_vec() -> Vec<NodeModel> {
        NodeModel::iter().collect()
    }

    /// Get the `NodeKind` for this model
    /// Uses the existing hardcoded mapping in `NodeConfig::get_model()`
    #[must_use]
    pub fn kind(&self) -> NodeKind {
        NodeConfig::get_model(*self).kind
    }

    /// The ZTP methods this model takes, most models one, two models two, and models with no
    /// config generator none. `Ztp::Disabled` is allowed for every model and is never listed.
    /// Every model is named, so a new one does not compile until it has an entry.
    #[must_use]
    pub fn ztp_methods(self) -> &'static [Ztp] {
        match self {
            NodeModel::CentosLinux
            | NodeModel::AlmaLinux
            | NodeModel::DevboxLinux
            | NodeModel::RockyLinux
            | NodeModel::FedoraLinux
            | NodeModel::OpensuseLinux
            | NodeModel::RedhatLinux
            | NodeModel::SuseLinux
            | NodeModel::UbuntuLinux
            | NodeModel::OmarchyLinux
            | NodeModel::NixosLinux
            | NodeModel::KaliLinux
            | NodeModel::InfrahubServer
            | NodeModel::JenkinsServer
            | NodeModel::NautobotServer
            | NodeModel::NetboxServer
            | NodeModel::SignozServer
            | NodeModel::VirtServer
            | NodeModel::GenericVm
            | NodeModel::FreeBsd
            | NodeModel::OpenBsd
            | NodeModel::DevboxWindows
            | NodeModel::WindowsServer
            | NodeModel::AlpineLinux => &[Ztp::CloudInit],
            NodeModel::AristaVeos | NodeModel::ArubaAoscx => &[Ztp::Tftp],
            NodeModel::JuniperVsrxv3 => &[Ztp::Tftp, Ztp::Cdrom],
            NodeModel::JuniperVevolved => &[Ztp::Tftp, Ztp::Usb],
            NodeModel::CiscoCsr1000v
            | NodeModel::CiscoCat8000v
            | NodeModel::CiscoCat9000v
            | NodeModel::CiscoAsav
            | NodeModel::CiscoNexus9300v
            | NodeModel::CiscoIosxrv9000
            | NodeModel::CiscoFtdv
            | NodeModel::JuniperVrouter
            | NodeModel::JuniperVswitch
            | NodeModel::PaloaltoPanos => &[Ztp::Cdrom],
            NodeModel::CiscoIosv
            | NodeModel::CiscoIosvl2
            | NodeModel::CiscoIse
            | NodeModel::MikrotikChr => &[Ztp::Disk],
            NodeModel::CumulusLinux => &[Ztp::Usb],
            NodeModel::SonicLinux => &[Ztp::Http],
            NodeModel::FlatcarLinux => &[Ztp::Ignition],
            NodeModel::TalosLinux => &[Ztp::NoCloud],
            NodeModel::AristaCeos
            | NodeModel::NokiaSrlinux
            | NodeModel::JuniperCevolved
            | NodeModel::FrrLinux => &[Ztp::Volume],
            NodeModel::GitlabCe
            | NodeModel::ForgejoForge
            | NodeModel::SurrealDb
            | NodeModel::MysqlDb
            | NodeModel::MongoDb
            | NodeModel::PostgresqlDb
            | NodeModel::NatsBus
            | NodeModel::HashicorpVault
            | NodeModel::KeycloakServer
            | NodeModel::GenericContainer
            | NodeModel::GenericUnikernel
            | NodeModel::UnikraftUnikernel
            | NodeModel::NanosUnikernel => &[],
        }
    }

    /// Whether nodes of this model accept SSH logins. Talos Linux has no SSH or shell.
    #[must_use]
    pub fn has_ssh(&self) -> bool {
        !matches!(self, NodeModel::TalosLinux)
    }

    /// Whether every data interface of a container of this model stays up, disabled ones too.
    /// cJunosEvolved waits at boot until all the data interfaces it is told about are up.
    #[must_use]
    pub fn keeps_data_interfaces_up(&self) -> bool {
        matches!(self, NodeModel::JuniperCevolved)
    }

    /// TCP port that shows a virtual machine of this model is ready.
    ///
    /// Talos Linux has no SSH, so its API port is used instead.
    #[must_use]
    pub fn ready_port(&self) -> u16 {
        match self {
            NodeModel::TalosLinux => TALOS_API_PORT,
            _ => SSH_PORT,
        }
    }

    /// Boot disk size in GB used when the manifest does not set `boot_disk.size`.
    ///
    /// `None` keeps the image's own size. The Talos Linux image is about 4 GB but Talos needs
    /// at least 10 GB.
    #[must_use]
    pub fn default_boot_disk_size(&self) -> Option<u16> {
        match self {
            NodeModel::TalosLinux => Some(TALOS_DEFAULT_BOOT_DISK_SIZE),
            _ => None,
        }
    }
}

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, EnumIter, JsonSchema)]
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
    Server,

    // Nokia
    Srlinux,

    // Palo Alto
    Panos,

    // Mikrotik
    Routeros,

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
            OsVariant::Server => write!(f, "server"),
            OsVariant::Srlinux => write!(f, "srlinux"),
            OsVariant::Panos => write!(f, "panos"),
            OsVariant::Routeros => write!(f, "routeros"),
            OsVariant::Unknown => write!(f, "unknown"),
        }
    }
}
impl OsVariant {
    #[must_use]
    pub fn to_vec() -> Vec<OsVariant> {
        OsVariant::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq, Eq, EnumIter, JsonSchema)]
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
    #[must_use]
    pub fn to_vec() -> Vec<CpuArchitecture> {
        CpuArchitecture::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq, Eq, EnumIter, JsonSchema)]
pub enum MachineType {
    #[default]
    #[serde(rename = "pc")]
    Pc, // alias of pc-i440fx-X.Y - Qemu version dependent
    #[serde(rename = "q35")]
    Q35, // alias of pc-q35-X.Y - Qemu version dependent
    #[serde(rename = "pc-q35-5.0")]
    PcQ35_5_0,
    #[serde(rename = "pc-q35-5.2")]
    PcQ35_5_2,
    #[serde(rename = "pc-q35-6.2")]
    PcQ35_6_2,
    #[serde(rename = "pc-q35-6.0")]
    PcQ35_6_0,
    #[serde(rename = "pc-q35-8.0")]
    PcQ35_8_0,
    #[serde(rename = "pc-q35-8.1")]
    PcQ35_8_1,
    #[serde(rename = "pc-q35-8.2")]
    PcQ35_8_2,
    #[serde(rename = "pc-i440fx-4.2")]
    PcI440Fx4_2,
    #[serde(rename = "pc-i440fx-5.1")]
    PcI440Fx5_1,
    #[serde(rename = "pc-i440fx-8.0")]
    PcI440Fx8_0,
    #[serde(rename = "pc-i440fx-8.1")]
    PcI440Fx8_1,
    #[serde(rename = "pc-i440fx-8.2")]
    PcI440Fx8_2,
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
            MachineType::PcI440Fx5_1 => write!(f, "pc-i440fx-5.1"),
            MachineType::PcI440Fx4_2 => write!(f, "pc-i440fx-4.2"),
            MachineType::PcI440Fx8_0 => write!(f, "pc-i440fx-8.0"),
            MachineType::PcI440Fx8_1 => write!(f, "pc-i440fx-8.1"),
            MachineType::PcI440Fx8_2 => write!(f, "pc-i440fx-8.2"),
        }
    }
}
impl MachineType {
    #[must_use]
    pub fn to_vec() -> Vec<MachineType> {
        MachineType::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, EnumIter, JsonSchema, PartialEq, Eq)]
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
    #[must_use]
    pub fn to_vec() -> Vec<InterfaceType> {
        InterfaceType::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq, Eq, EnumIter, JsonSchema)]
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
    #[must_use]
    pub fn to_vec() -> Vec<BiosTypes> {
        BiosTypes::iter().collect()
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq, EnumIter, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnikernelBootMode {
    #[default]
    DirectKernel,
    DiskBoot,
}
impl fmt::Display for UnikernelBootMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnikernelBootMode::DirectKernel => write!(f, "direct_kernel"),
            UnikernelBootMode::DiskBoot => write!(f, "disk_boot"),
        }
    }
}
impl std::str::FromStr for UnikernelBootMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "direct_kernel" => Ok(UnikernelBootMode::DirectKernel),
            "disk_boot" => Ok(UnikernelBootMode::DiskBoot),
            _ => Err(format!("Unknown unikernel boot mode: {s}")),
        }
    }
}
impl UnikernelBootMode {
    #[must_use]
    pub fn to_vec() -> Vec<UnikernelBootMode> {
        UnikernelBootMode::iter().collect()
    }
}

/// Zero touch provisioning for a node: off, or the one method Sherpa provisions it by at boot.
/// The image, the API, the database, the manifest and the web form all carry this type and
/// spell it the same way. Which methods a model takes is `NodeModel::ztp_methods`.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    Deserialize,
    Serialize,
    Display,
    EnumIter,
    JsonSchema,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Ztp {
    /// Sherpa provisions nothing.
    #[default]
    Disabled,
    #[serde(rename = "cloud-init")]
    #[strum(to_string = "cloud-init")]
    CloudInit,
    Cdrom,
    Disk,
    Http,
    Ignition,
    /// `NoCloud` `cidata` CD-ROM carrying the node's own `user-data` (if any) with Sherpa's
    /// `meta-data` and static management `network-config`.
    NoCloud,
    Tftp,
    Usb,
    /// Config files mounted into a container.
    Volume,
}

#[derive(
    Clone, Debug, Deserialize, Default, Serialize, PartialEq, Eq, Hash, EnumIter, JsonSchema,
)]
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
impl std::str::FromStr for NodeKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "virtual_machine" => Ok(NodeKind::VirtualMachine),
            "container" => Ok(NodeKind::Container),
            "unikernel" => Ok(NodeKind::Unikernel),
            _ => Err(format!("Unknown node kind: {s}")),
        }
    }
}
impl NodeKind {
    #[must_use]
    pub fn to_vec() -> Vec<NodeKind> {
        NodeKind::iter().collect()
    }
}

/// A node's place in its lifecycle. The allowed moves between states are in
/// `node_lifecycle`: operations move a node with an `OperationEvent`, the scanner with an
/// `Observation`. Nothing else writes it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default, EnumIter, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum NodeState {
    /// An operation (`up` or a redeploy) is creating the node's domain or container.
    Building,

    /// The node's domain or container is started, and the node is not ready yet.
    Starting,

    /// The node is ready: its readiness check passed, or it has none.
    Running,

    /// The node's domain or container exists and is not running.
    Stopped,

    /// An operation on the node failed, or its runtime reports a fault.
    Failed,

    /// The node has a record but no domain or container.
    Missing,

    /// A state this build does not know, such as one written by a newer version, or a node
    /// recorded before the lifecycle was explicit. Unknown text decodes to it, so one such row
    /// does not fail a whole listing.
    #[default]
    #[serde(other)]
    Unknown,
}

impl fmt::Display for NodeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeState::Building => write!(f, "building"),
            NodeState::Starting => write!(f, "starting"),
            NodeState::Running => write!(f, "running"),
            NodeState::Stopped => write!(f, "stopped"),
            NodeState::Failed => write!(f, "failed"),
            NodeState::Missing => write!(f, "missing"),
            NodeState::Unknown => write!(f, "unknown"),
        }
    }
}

impl std::str::FromStr for NodeState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "building" => Ok(NodeState::Building),
            "starting" => Ok(NodeState::Starting),
            "running" => Ok(NodeState::Running),
            "stopped" => Ok(NodeState::Stopped),
            "failed" => Ok(NodeState::Failed),
            "missing" => Ok(NodeState::Missing),
            _ => Ok(NodeState::Unknown), // Graceful degradation
        }
    }
}

impl NodeState {
    #[must_use]
    pub fn to_vec() -> Vec<NodeState> {
        NodeState::iter().collect()
    }
}

/// Zero touch provisioning for a node image: off or the method Sherpa provisions its nodes by,
/// and with which credentials.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZtpSettings {
    /// `disabled`, or how the image's nodes are provisioned at boot.
    pub method: Ztp,
    /// The account the provisioning sets up, when the image has a fixed one.
    pub username: Option<String>,
    /// That account's password.
    pub password: Option<String>,
    /// Whether the account can log in with its password, as well as with keys.
    pub password_auth: bool,
}

/// A node image's virtual CPUs.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CpuSettings {
    /// The number of vCPUs.
    pub count: u8,
    pub architecture: CpuArchitecture,
    pub model: CpuModels,
    /// Whether the guest sees the host's hardware virtualisation, to run VMs of its own.
    pub vmx: bool,
}

impl Default for CpuSettings {
    fn default() -> Self {
        Self {
            count: 1,
            architecture: CpuArchitecture::default(),
            model: CpuModels::default(),
            vmx: false,
        }
    }
}

/// The machine a node image's guest runs on: the QEMU machine type and the firmware.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MachineSettings {
    /// The QEMU machine type.
    pub kind: MachineType,
    pub bios: BiosTypes,
}

/// The disk a node image boots from.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BootDiskSettings {
    /// The bus the boot disk attaches to.
    pub bus: DiskBuses,
}

/// The CD-ROM a node image boots with, if any, and the bus CD-ROMs attach to.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdromSettings {
    /// The file name of a CD-ROM image shipped with the node image.
    pub image: Option<String>,
    pub bus: DiskBuses,
}

/// A node image's network interfaces: how many, of which type, and how they are named.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InterfaceSettings {
    /// The number of data interfaces, not counting the management interface.
    pub data_count: u8,
    /// The interface device for a VM, or the network mode for a container.
    pub kind: InterfaceType,
    pub mtu: u16,
    pub prefix: String,
    pub first_index: u8,
    pub management: MgmtInterfaces,
    /// Interfaces the node holds back from the data interfaces a lab can link.
    pub reserved_count: u8,
}

impl Default for InterfaceSettings {
    fn default() -> Self {
        Self {
            data_count: 0,
            kind: InterfaceType::default(),
            mtu: 1500,
            prefix: "eth".to_owned(),
            first_index: 0,
            management: MgmtInterfaces::default(),
            reserved_count: 0,
        }
    }
}

/// How a unikernel image boots.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnikernelSettings {
    pub boot_mode: UnikernelBootMode,
}

/// A node image: the model and version, and how Sherpa builds and boots its nodes. The
/// settings are grouped, and each group is a nested object.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    #[schemars(skip)]
    pub id: Option<RecordId>,
    pub model: NodeModel,
    pub version: String,
    pub repo: Option<String>,
    pub os_variant: OsVariant,
    pub kind: NodeKind,
    /// Whether this version is the model's default.
    pub default: bool,
    /// The guest's memory, in MiB.
    pub memory: u16,
    pub cpu: CpuSettings,
    pub machine: MachineSettings,
    pub boot_disk: BootDiskSettings,
    pub cdrom: CdromSettings,
    pub interfaces: InterfaceSettings,
    pub ztp: ZtpSettings,
    /// How a unikernel boots; absent for every other kind of node.
    pub unikernel: Option<UnikernelSettings>,
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
            default: false,
            memory: 1024,
            cpu: CpuSettings::default(),
            machine: MachineSettings::default(),
            boot_disk: BootDiskSettings::default(),
            cdrom: CdromSettings::default(),
            interfaces: InterfaceSettings::default(),
            ztp: ZtpSettings::default(),
            unikernel: None,
        }
    }
}

impl NodeConfig {
    #[must_use]
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
            NodeModel::JuniperCevolved => NodeConfig::juniper_cevolved(),
            NodeModel::JuniperVsrxv3 => NodeConfig::juniper_vsrxv3(),

            // Nokia
            NodeModel::NokiaSrlinux => NodeConfig::nokia_srlinux(),

            // Palo Alto
            NodeModel::PaloaltoPanos => NodeConfig::paloalto_panos(),

            // Mikrotik
            NodeModel::MikrotikChr => NodeConfig::mikrotik_chr(),

            // FRR
            NodeModel::FrrLinux => NodeConfig::frr_linux(),

            // Linux
            NodeModel::AlmaLinux => NodeConfig::alma_linux(),
            NodeModel::RockyLinux => NodeConfig::rocky_linux(),
            NodeModel::AlpineLinux => NodeConfig::alpine_linux(),
            NodeModel::CumulusLinux => NodeConfig::cumulus_linux(),
            NodeModel::CentosLinux => NodeConfig::centos_linux(),
            NodeModel::DevboxLinux => NodeConfig::devbox_linux(),
            NodeModel::FedoraLinux => NodeConfig::fedora_linux(),
            NodeModel::RedhatLinux => NodeConfig::redhat_linux(),
            NodeModel::OpensuseLinux => NodeConfig::opensuse_linux(),
            NodeModel::SuseLinux => NodeConfig::suse_linux(),
            NodeModel::UbuntuLinux => NodeConfig::ubuntu_linux(),
            NodeModel::OmarchyLinux => NodeConfig::omarchy_linux(),
            NodeModel::NixosLinux => NodeConfig::nixos_linux(),
            NodeModel::KaliLinux => NodeConfig::kali_linux(),
            NodeModel::SonicLinux => NodeConfig::sonic_linux(),
            NodeModel::FlatcarLinux => NodeConfig::flatcar_linux(),
            NodeModel::TalosLinux => NodeConfig::talos_linux(),

            // BSD
            NodeModel::FreeBsd => NodeConfig::free_bsd(),
            NodeModel::OpenBsd => NodeConfig::open_bsd(),

            // Windows
            NodeModel::DevboxWindows => NodeConfig::devbox_windows(),
            NodeModel::WindowsServer => NodeConfig::windows_server(),

            // Application
            NodeModel::ForgejoForge => NodeConfig::forgejo_forge(),
            NodeModel::GitlabCe => NodeConfig::gitlab_ce(),
            NodeModel::InfrahubServer => NodeConfig::infrahub_server(),
            NodeModel::JenkinsServer => NodeConfig::jenkins_server(),
            NodeModel::NautobotServer => NodeConfig::nautobot_server(),
            NodeModel::NetboxServer => NodeConfig::netbox_server(),
            NodeModel::SignozServer => NodeConfig::signoz_server(),
            NodeModel::VirtServer => NodeConfig::virt_server(),

            // Messaging
            NodeModel::NatsBus => NodeConfig::nats_bus(),

            // SQL
            NodeModel::MysqlDb => NodeConfig::mysql_db(),
            NodeModel::MongoDb => NodeConfig::mongo_db(),
            NodeModel::PostgresqlDb => NodeConfig::postgresql_db(),
            NodeModel::SurrealDb => NodeConfig::surreal_db(),

            // HashiCorp
            NodeModel::HashicorpVault => NodeConfig::hashicorp_vault(),

            // Identity
            NodeModel::KeycloakServer => NodeConfig::keycloak_server(),

            // Generic
            NodeModel::GenericContainer => NodeConfig::generic_container(),
            NodeModel::GenericUnikernel => NodeConfig::generic_unikernel(),
            NodeModel::GenericVm => NodeConfig::generic_vm(),

            // Unikernels
            NodeModel::UnikraftUnikernel => NodeConfig::unikraft_unikernel(),
            NodeModel::NanosUnikernel => NodeConfig::nanos_unikernel(),
        }
    }

    /// The repo a container node runs, for the node's version as pulled (`self`): the
    /// manifest's `image`, or else the repo its version was pulled from or imported as; `None`
    /// if it has neither. A model with a built-in repo is treated the same way: its built-in
    /// repo is only where its records start.
    #[must_use]
    pub fn container_repo(&self, manifest_image: Option<&str>) -> Option<String> {
        manifest_image
            .map(str::to_string)
            .or_else(|| self.repo.clone())
    }
    #[must_use]
    pub fn arista_veos() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::AristaVeos,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Eos,
            kind: NodeKind::VirtualMachine,
            memory: 2048,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: Some("aboot.iso".to_owned()),
                bus: DiskBuses::Ide,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Tftp,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 52,
                prefix: "Eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
                first_index: 1,
                management: MgmtInterfaces::Management1,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn arista_ceos() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::AristaCeos,
            version: "0.0.0".to_owned(),
            repo: Some(CONTAINER_ARISTA_CEOS_REPO.to_string()),
            os_variant: OsVariant::Eos,
            kind: NodeKind::Container,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Volume,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 52,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
                first_index: 1,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn aruba_aoscx() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::ArubaAoscx,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Aos,
            kind: NodeKind::VirtualMachine,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Tftp,
                username: None,
                password: None,
                password_auth: true,
            },
            interfaces: InterfaceSettings {
                data_count: 52,
                prefix: "1/1/".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 1,
                management: MgmtInterfaces::Mgmt,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_asav() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoAsav,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Asa,
            kind: NodeKind::VirtualMachine,
            memory: 2048,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Ide,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 8,
                prefix: "GigabitEthernet0".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 1,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_csr1000v() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoCsr1000v,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Iosxe,
            kind: NodeKind::VirtualMachine,
            memory: 3072,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: true,
            },
            interfaces: InterfaceSettings {
                data_count: 15,
                prefix: "Gig".to_owned(),
                kind: InterfaceType::Vmxnet3,
                mtu: MTU_JUMBO_INT,
                first_index: 1,
                management: MgmtInterfaces::GigabitEthernet1,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_cat8000v() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoCat8000v,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Iosxe,
            kind: NodeKind::VirtualMachine,
            memory: 4096,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 15,
                prefix: "Gig".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::GigabitEthernet1,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_cat9000v() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoCat9000v,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Iosxe,
            kind: NodeKind::VirtualMachine,
            memory: 18432,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 8,
                prefix: "Gig0/0/".to_owned(),
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
                first_index: 1,
                management: MgmtInterfaces::GigabitEthernet0_0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_iosxrv9000() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoIosxrv9000,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Iosxr,
            kind: NodeKind::VirtualMachine,
            memory: 20480,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::Uefi,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 31,
                prefix: "Gig0/0/0/".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::MgmtEth0Rp0Cpu0_0,
                reserved_count: 2,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_nexus9300v() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoNexus9300v,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Nxos,
            kind: NodeKind::VirtualMachine,
            memory: 12288,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::Uefi,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Ide,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 64,
                prefix: "Eth1/".to_owned(),
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
                first_index: 1,
                management: MgmtInterfaces::Mgmt0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_iosv() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoIosv,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Ios,
            kind: NodeKind::VirtualMachine,
            memory: 768,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Virtio,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Disk,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 15,
                prefix: "Gig0/".to_owned(),
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::GigabitEthernet0_0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_iosvl2() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoIosvl2,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Ios,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Virtio,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Disk,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                // IOSvL2 crashes with more than 16 NICs: GigabitEthernet0/0 for management plus 15 data.
                data_count: 15,
                prefix: "Gig".to_owned(),
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::GigabitEthernet0_0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_ise() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoIse,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Ise,
            kind: NodeKind::VirtualMachine,
            memory: 16384,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Disk,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cisco_ftdv() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CiscoFtdv,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Fxos,
            kind: NodeKind::VirtualMachine,
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 8,
                prefix: "gig0/".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Management0_0,
                reserved_count: 1,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn juniper_vrouter() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JuniperVrouter,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Junos,
            kind: NodeKind::VirtualMachine,
            memory: 5120,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::IvyBridge,
                vmx: true,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 10,
                prefix: "ge-0/0/".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
                first_index: 0,
                management: MgmtInterfaces::Fxp0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn juniper_vswitch() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JuniperVswitch,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Junos,
            kind: NodeKind::VirtualMachine,
            memory: 5120,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::IvyBridge,
                vmx: true,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 10,
                prefix: "ge-0/0/".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
                first_index: 0,
                management: MgmtInterfaces::Fxp0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn juniper_vevolved() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JuniperVevolved,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Junos,
            kind: NodeKind::VirtualMachine,
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::IvyBridge,
                vmx: true,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::Uefi,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Tftp,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 12,
                prefix: "et-0/0/".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
                first_index: 0,
                management: MgmtInterfaces::Re0Mgmt0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn juniper_vsrxv3() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JuniperVsrxv3,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Junos,
            kind: NodeKind::VirtualMachine,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::SandyBridge,
                vmx: true,
            },
            machine: MachineSettings {
                kind: MachineType::PcI440Fx8_0,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Ide,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 8,
                prefix: "ge-0/0/".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
                first_index: 0,
                management: MgmtInterfaces::Fxp0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn alma_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::AlmaLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn rocky_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::RockyLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::Uefi,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn alpine_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::AlpineLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 2,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn cumulus_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CumulusLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            kind: NodeKind::VirtualMachine,
            os_variant: OsVariant::Nvue,
            memory: 2048,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Usb,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 52,
                prefix: "swp".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 1,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    /// cJunosEvolved. The vCPUs and memory are its EVO VM's, given to the container as
    /// `CPTX_EVOVM_CPU` and `CPTX_EVOVM_MEM_MB`; they are cJunosEvolved's minimums.
    #[must_use]
    pub fn juniper_cevolved() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JuniperCevolved,
            version: "0.0.0".to_owned(),
            repo: Some(CONTAINER_JUNIPER_CEVOLVED_REPO.to_string()),
            os_variant: OsVariant::Junos,
            kind: NodeKind::Container,
            memory: 6292,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Volume,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 12,
                prefix: "et-0/0/".to_owned(),
                kind: InterfaceType::default(),
                mtu: MTU_JUMBO_INT,
                first_index: 1,
                management: MgmtInterfaces::Re0Mgmt0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn nokia_srlinux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::NokiaSrlinux,
            version: "0.0.0".to_owned(),
            repo: Some(CONTAINER_NOKIA_SRLINUX_REPO.to_string()),
            os_variant: OsVariant::Srlinux,
            kind: NodeKind::Container,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Volume,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 34,
                prefix: "eth-1/".to_owned(),
                kind: InterfaceType::default(),
                mtu: MTU_JUMBO_INT,
                first_index: 1,
                management: MgmtInterfaces::Mgmt0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn centos_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::CentosLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn devbox_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::DevboxLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn fedora_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::FedoraLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn redhat_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::RedhatLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn suse_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::SuseLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn opensuse_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::OpensuseLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn ubuntu_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::UbuntuLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    /// Talos Linux: immutable, API-managed Kubernetes OS with no SSH or shell.
    ///
    /// The machine config is delivered as `NoCloud` `user-data` on a `cidata` CD-ROM.
    #[must_use]
    pub fn talos_linux() -> NodeConfig {
        let ubuntu = NodeConfig::ubuntu_linux();
        NodeConfig {
            model: NodeModel::TalosLinux,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                ..ubuntu.cpu
            },
            ztp: ZtpSettings {
                method: Ztp::NoCloud,
                ..ubuntu.ztp
            },
            ..ubuntu
        }
    }
    /// NixOS, built from the image recipe in `bwks/nixos-image`. More memory than the other
    /// distros, because rebuilding the system runs Nix on the node.
    #[tracing::instrument(level = "debug")]
    pub fn nixos_linux() -> NodeConfig {
        let ubuntu = NodeConfig::ubuntu_linux();
        NodeConfig {
            model: NodeModel::NixosLinux,
            memory: 2048,
            cpu: CpuSettings {
                count: 2,
                ..ubuntu.cpu
            },
            ..ubuntu
        }
    }
    #[tracing::instrument(level = "debug")]
    pub fn omarchy_linux() -> NodeConfig {
        let ubuntu = NodeConfig::ubuntu_linux();
        NodeConfig {
            model: NodeModel::OmarchyLinux,
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                ..ubuntu.cpu
            },
            machine: MachineSettings {
                bios: BiosTypes::Uefi,
                ..ubuntu.machine
            },
            interfaces: InterfaceSettings {
                mtu: MTU_STD,
                ..ubuntu.interfaces
            },
            ..ubuntu
        }
    }
    #[must_use]
    pub fn kali_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::KaliLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn sonic_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::SonicLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Virtio,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Http,
                username: None,
                password: None,
                password_auth: true,
            },
            interfaces: InterfaceSettings {
                data_count: 52,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 1,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn flatcar_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::FlatcarLinux,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 2048,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Ignition,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn free_bsd() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::FreeBsd,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Bsd,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn open_bsd() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::OpenBsd,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Bsd,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn devbox_windows() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::DevboxWindows,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Server,
            kind: NodeKind::VirtualMachine,
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn windows_server() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::WindowsServer,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Server,
            kind: NodeKind::VirtualMachine,
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn jenkins_server() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::JenkinsServer,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: true,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn nautobot_server() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::NautobotServer,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: true,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn virt_server() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::VirtServer,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: true,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn netbox_server() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::NetboxServer,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: true,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn infrahub_server() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::InfrahubServer,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 12288,
            cpu: CpuSettings {
                count: 6,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: true,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn signoz_server() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::SignozServer,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Linux,
            kind: NodeKind::VirtualMachine,
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: true,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn forgejo_forge() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::ForgejoForge,
            repo: Some(CONTAINER_FORGEJO_REPO.to_string()),
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn gitlab_ce() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::GitlabCe,
            repo: Some(CONTAINER_GITLAB_CE_REPO.to_string()),
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn surreal_db() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::SurrealDb,
            repo: Some(CONTAINER_SURREAL_DB_REPO.to_string()),
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn hashicorp_vault() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::HashicorpVault,
            repo: Some(CONTAINER_HASHICORP_VAULT_REPO.to_string()),
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn keycloak_server() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::KeycloakServer,
            repo: Some(CONTAINER_KEYCLOAK_REPO.to_string()),
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 2048,
            cpu: CpuSettings {
                count: 2,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn mysql_db() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::MysqlDb,
            repo: None,
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn mongo_db() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::MongoDb,
            repo: Some(CONTAINER_MONGO_DB_REPO.to_string()),
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn nats_bus() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::NatsBus,
            repo: Some(CONTAINER_NATS_REPO.to_string()),
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 512,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn postgresql_db() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::PostgresqlDb,
            repo: None,
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn mikrotik_chr() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::MikrotikChr,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Routeros,
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Ide,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Ide,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Disk,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 8,
                prefix: "ether".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
                first_index: 1,
                management: MgmtInterfaces::Ether1,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn paloalto_panos() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::PaloaltoPanos,
            version: "0.0.0".to_owned(),
            repo: None,
            os_variant: OsVariant::Panos,
            kind: NodeKind::VirtualMachine,
            memory: 8192,
            cpu: CpuSettings {
                count: 2,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Virtio,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Cdrom,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 24,
                prefix: "ethernet1/".to_owned(),
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
                first_index: 0,
                management: MgmtInterfaces::Management,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn frr_linux() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::FrrLinux,
            version: "0.0.0".to_owned(),
            repo: Some(CONTAINER_FRR_REPO.to_string()),
            os_variant: OsVariant::Linux,
            kind: NodeKind::Container,
            memory: 512,
            cpu: CpuSettings {
                count: 1,
                architecture: CpuArchitecture::X86_64,
                model: CpuModels::HostModel,
                vmx: false,
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                bios: BiosTypes::SeaBios,
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            unikernel: None,
            ztp: ZtpSettings {
                method: Ztp::Volume,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 8,
                prefix: "eth".to_owned(),
                kind: InterfaceType::default(),
                mtu: MTU_STD,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            default: true,
        }
    }
    #[must_use]
    pub fn generic_container() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::GenericContainer,
            repo: None,
            version: "0.0.0".to_owned(),
            kind: NodeKind::Container,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::MacVlan,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn generic_unikernel() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::GenericUnikernel,
            repo: None,
            version: "0.0.0".to_owned(),
            kind: NodeKind::Unikernel,
            memory: 512,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            unikernel: Some(UnikernelSettings {
                boot_mode: UnikernelBootMode::DirectKernel,
            }),
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn unikraft_unikernel() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::UnikraftUnikernel,
            repo: None,
            version: "0.0.0".to_owned(),
            kind: NodeKind::Unikernel,
            memory: 512,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            unikernel: Some(UnikernelSettings {
                boot_mode: UnikernelBootMode::DirectKernel,
            }),
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn nanos_unikernel() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::NanosUnikernel,
            repo: None,
            version: "0.0.0".to_owned(),
            kind: NodeKind::Unikernel,
            memory: 512,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            unikernel: Some(UnikernelSettings {
                boot_mode: UnikernelBootMode::DiskBoot,
            }),
            ztp: ZtpSettings {
                method: Ztp::Disabled,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
    #[must_use]
    pub fn generic_vm() -> NodeConfig {
        NodeConfig {
            id: None,
            model: NodeModel::GenericVm,
            repo: None,
            version: "0.0.0".to_owned(),
            kind: NodeKind::VirtualMachine,
            memory: 1024,
            cpu: CpuSettings {
                count: 1,
                ..CpuSettings::default()
            },
            cdrom: CdromSettings {
                image: None,
                bus: DiskBuses::Sata,
            },
            ztp: ZtpSettings {
                method: Ztp::CloudInit,
                username: None,
                password: None,
                password_auth: false,
            },
            interfaces: InterfaceSettings {
                data_count: 1,
                prefix: "eth".to_owned(),
                kind: InterfaceType::Virtio,
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
                ..InterfaceSettings::default()
            },
            default: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pulled(model: NodeModel, repo: Option<&str>) -> NodeConfig {
        let mut config = NodeConfig::get_model(model);
        config.repo = repo.map(str::to_string);
        config
    }

    #[test]
    fn test_container_repo_of_a_model_with_a_built_in_repo_prefers_the_manifest_image() {
        let config = pulled(NodeModel::MongoDb, Some("mongo"));
        assert_eq!(
            config.container_repo(Some("mirror.example/mongo")),
            Some("mirror.example/mongo".to_string())
        );
    }

    #[test]
    fn test_container_repo_of_a_model_with_a_built_in_repo_runs_the_repo_it_was_pulled_from() {
        let config = pulled(NodeModel::NatsBus, Some("docker.io/library/nats"));
        assert_eq!(
            config.container_repo(None),
            Some("docker.io/library/nats".to_string())
        );
    }

    #[test]
    fn test_container_repo_of_a_model_with_a_built_in_repo_and_no_recorded_repo_is_none() {
        let config = pulled(NodeModel::FrrLinux, None);
        assert_eq!(config.container_repo(None), None);
    }

    #[test]
    fn test_container_repo_prefers_the_manifest_image() {
        let config = pulled(NodeModel::GenericContainer, Some("alpine"));
        assert_eq!(
            config.container_repo(Some("ubuntu")),
            Some("ubuntu".to_string())
        );
    }

    #[test]
    fn test_container_repo_falls_back_to_the_pulled_repo() {
        let config = pulled(NodeModel::PostgresqlDb, Some("postgres"));
        assert_eq!(config.container_repo(None), Some("postgres".to_string()));
    }

    #[test]
    fn test_container_repo_without_image_or_pulled_repo_is_none() {
        let config = pulled(NodeModel::MysqlDb, None);
        assert_eq!(config.container_repo(None), None);
    }

    #[test]
    fn test_all_variants_are_unique() {
        use std::collections::HashSet;
        let variants = NodeModel::to_vec();
        let unique: HashSet<String> = variants
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        assert_eq!(
            variants.len(),
            unique.len(),
            "all_variants contains duplicates"
        );
    }

    // =========================================================================
    // UnikernelBootMode enum tests
    // =========================================================================

    #[test]
    fn test_unikernel_boot_mode_display() {
        assert_eq!(UnikernelBootMode::DirectKernel.to_string(), "direct_kernel");
        assert_eq!(UnikernelBootMode::DiskBoot.to_string(), "disk_boot");
    }

    #[test]
    fn test_unikernel_boot_mode_from_str() {
        assert_eq!(
            "direct_kernel".parse::<UnikernelBootMode>().unwrap(),
            UnikernelBootMode::DirectKernel
        );
        assert_eq!(
            "disk_boot".parse::<UnikernelBootMode>().unwrap(),
            UnikernelBootMode::DiskBoot
        );
        assert!("invalid".parse::<UnikernelBootMode>().is_err());
    }

    #[test]
    fn test_unikernel_boot_mode_serde_roundtrip() {
        let modes = vec![UnikernelBootMode::DirectKernel, UnikernelBootMode::DiskBoot];
        for mode in modes {
            let json = serde_json::to_string(&mode).unwrap();
            let deserialized: UnikernelBootMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, deserialized);
        }
    }

    #[test]
    fn test_unikernel_boot_mode_default() {
        assert_eq!(
            UnikernelBootMode::default(),
            UnikernelBootMode::DirectKernel
        );
    }

    #[test]
    fn test_unikernel_boot_mode_to_vec() {
        let variants = UnikernelBootMode::to_vec();
        assert_eq!(variants.len(), 2);
        assert!(variants.contains(&UnikernelBootMode::DirectKernel));
        assert!(variants.contains(&UnikernelBootMode::DiskBoot));
    }

    // =========================================================================
    // Unikernel NodeModel variant tests
    // =========================================================================

    #[test]
    fn test_unikernel_model_display_and_from_str() {
        let models = vec![
            (NodeModel::GenericUnikernel, "generic_unikernel"),
            (NodeModel::UnikraftUnikernel, "unikraft_unikernel"),
            (NodeModel::NanosUnikernel, "nanos_unikernel"),
        ];
        for (model, expected_str) in models {
            assert_eq!(model.to_string(), expected_str);
            assert_eq!(expected_str.parse::<NodeModel>().unwrap(), model);
        }
    }

    #[test]
    fn test_unikernel_model_kind_mapping() {
        let unikernel_models = vec![
            NodeModel::GenericUnikernel,
            NodeModel::UnikraftUnikernel,
            NodeModel::NanosUnikernel,
        ];
        for model in unikernel_models {
            let config = NodeConfig::get_model(model);
            assert_eq!(
                config.kind,
                NodeKind::Unikernel,
                "Model {model} should be Unikernel kind"
            );
        }
    }

    #[test]
    fn test_unikernel_config_boot_modes() {
        // DirectKernel models
        for model in [NodeModel::GenericUnikernel, NodeModel::UnikraftUnikernel] {
            let config = NodeConfig::get_model(model);
            assert_eq!(
                config.unikernel.map(|u| u.boot_mode),
                Some(UnikernelBootMode::DirectKernel),
                "Model {model} should default to DirectKernel"
            );
        }

        // DiskBoot model
        let nanos_config = NodeConfig::get_model(NodeModel::NanosUnikernel);
        assert_eq!(
            nanos_config.unikernel.map(|u| u.boot_mode),
            Some(UnikernelBootMode::DiskBoot),
            "NanosUnikernel should default to DiskBoot"
        );
    }

    #[test]
    fn test_unikernel_config_no_ztp() {
        let unikernel_models = vec![
            NodeModel::GenericUnikernel,
            NodeModel::UnikraftUnikernel,
            NodeModel::NanosUnikernel,
        ];
        for model in unikernel_models {
            let config = NodeConfig::get_model(model);
            assert_eq!(
                config.ztp.method,
                Ztp::Disabled,
                "Model {model} should have ZTP disabled"
            );
        }
    }

    #[test]
    fn test_unikernel_config_defaults() {
        let config = NodeConfig::get_model(NodeModel::UnikraftUnikernel);
        assert_eq!(config.memory, 512);
        assert_eq!(config.cpu.count, 1);
        assert_eq!(config.interfaces.kind.to_string(), "virtio");
    }

    #[test]
    fn test_non_unikernel_models_have_no_boot_mode() {
        let vm_models = vec![
            NodeModel::CiscoNexus9300v,
            NodeModel::AristaVeos,
            NodeModel::UbuntuLinux,
        ];
        for model in vm_models {
            let config = NodeConfig::get_model(model);
            assert!(
                config.unikernel.is_none(),
                "VM model {model} should not have unikernel settings"
            );
        }
    }
    #[test]
    fn test_omarchy_model_defaults() {
        let model: super::NodeModel = "omarchy_linux".parse().unwrap();
        let config = super::NodeConfig::get_model(model);
        assert_eq!(model.to_string(), "omarchy_linux");
        assert!(matches!(config.machine.bios, super::BiosTypes::Uefi));
        assert_eq!(config.ztp.method, super::Ztp::CloudInit);
        assert!(matches!(config.boot_disk.bus, super::DiskBuses::Virtio));
        assert!(config.cdrom.image.is_none());
        assert_eq!(config.memory, 8192);
        assert_eq!(config.cpu.count, 4);
        let encoded = serde_json::to_string(&model).unwrap();
        assert_eq!(
            serde_json::from_str::<super::NodeModel>(&encoded).unwrap(),
            model
        );
    }

    #[test]
    fn test_nixos_model_defaults() {
        let model: super::NodeModel = "nixos_linux".parse().unwrap();
        let config = super::NodeConfig::get_model(model);
        assert_eq!(model.to_string(), "nixos_linux");
        assert_eq!(config.kind, super::NodeKind::VirtualMachine);
        assert!(matches!(config.os_variant, super::OsVariant::Linux));
        assert!(matches!(config.machine.bios, super::BiosTypes::SeaBios));
        assert_eq!(config.ztp.method, super::Ztp::CloudInit);
        assert!(matches!(config.boot_disk.bus, super::DiskBuses::Virtio));
        assert!(matches!(
            config.interfaces.kind,
            super::InterfaceType::Virtio
        ));
        assert_eq!(config.interfaces.prefix, "eth");
        assert!(matches!(
            config.interfaces.management,
            super::MgmtInterfaces::Eth0
        ));
        assert!(config.cdrom.image.is_none());
        assert_eq!(config.cpu.count, 2);
        assert_eq!(config.memory, 2048);
        let encoded = serde_json::to_string(&model).unwrap();
        assert_eq!(
            serde_json::from_str::<super::NodeModel>(&encoded).unwrap(),
            model
        );
    }

    #[test]
    fn talos_linux_model_defaults() {
        let model: super::NodeModel = "talos_linux".parse().unwrap();
        let config = super::NodeConfig::get_model(model);
        assert_eq!(model.to_string(), "talos_linux");
        assert_eq!(config.kind, super::NodeKind::VirtualMachine);
        assert!(matches!(config.os_variant, super::OsVariant::Linux));
        assert!(matches!(config.machine.kind, super::MachineType::Q35));
        assert!(matches!(config.cpu.model, super::CpuModels::HostModel));
        assert_eq!(config.ztp.method, super::Ztp::NoCloud);
        assert!(matches!(config.boot_disk.bus, super::DiskBuses::Virtio));
        assert!(matches!(config.cdrom.bus, super::DiskBuses::Sata));
        assert!(matches!(
            config.interfaces.kind,
            super::InterfaceType::Virtio
        ));
        assert!(config.cdrom.image.is_none());
        assert_eq!(config.cpu.count, 2);
        assert_eq!(config.memory, 4096);
        assert!(!config.ztp.password_auth);
        assert_eq!(model.default_boot_disk_size(), Some(20));
        assert!(!model.has_ssh());
        assert_eq!(model.ready_port(), TALOS_API_PORT);
        let encoded = serde_json::to_string(&model).unwrap();
        assert_eq!(
            serde_json::from_str::<super::NodeModel>(&encoded).unwrap(),
            model
        );
    }

    #[test]
    fn other_vm_models_keep_ssh_readiness_and_image_disk_size() {
        assert_eq!(super::NodeModel::UbuntuLinux.ready_port(), SSH_PORT);
        assert_eq!(super::NodeModel::FlatcarLinux.ready_port(), SSH_PORT);
        assert_eq!(super::NodeModel::UbuntuLinux.default_boot_disk_size(), None);
        assert!(super::NodeModel::UbuntuLinux.has_ssh());
    }

    #[test]
    fn a_node_state_this_build_does_not_know_decodes_as_unknown() {
        // M1.8: one row written by another version must not fail a whole listing.
        for text in ["\"created\"", "\"hibernating\""] {
            let state: NodeState = serde_json::from_str(text).unwrap();
            assert_eq!(state, NodeState::Unknown, "{text}");
        }
        for state in NodeState::iter() {
            let text = serde_json::to_string(&state).unwrap();
            assert_eq!(serde_json::from_str::<NodeState>(&text).unwrap(), state);
            assert_eq!(text, format!("\"{state}\""));
        }
    }

    /// The methods each model takes, written out: the generators' model arms.
    fn expected_ztp_methods(model: NodeModel) -> Vec<Ztp> {
        match model {
            NodeModel::CentosLinux
            | NodeModel::AlmaLinux
            | NodeModel::DevboxLinux
            | NodeModel::RockyLinux
            | NodeModel::FedoraLinux
            | NodeModel::OpensuseLinux
            | NodeModel::RedhatLinux
            | NodeModel::SuseLinux
            | NodeModel::UbuntuLinux
            | NodeModel::OmarchyLinux
            | NodeModel::NixosLinux
            | NodeModel::KaliLinux
            | NodeModel::InfrahubServer
            | NodeModel::JenkinsServer
            | NodeModel::NautobotServer
            | NodeModel::NetboxServer
            | NodeModel::SignozServer
            | NodeModel::VirtServer
            | NodeModel::GenericVm
            | NodeModel::FreeBsd
            | NodeModel::OpenBsd
            | NodeModel::DevboxWindows
            | NodeModel::WindowsServer
            | NodeModel::AlpineLinux => vec![Ztp::CloudInit],
            NodeModel::AristaVeos | NodeModel::ArubaAoscx => vec![Ztp::Tftp],
            NodeModel::JuniperVsrxv3 => vec![Ztp::Tftp, Ztp::Cdrom],
            NodeModel::JuniperVevolved => vec![Ztp::Tftp, Ztp::Usb],
            NodeModel::CiscoCsr1000v
            | NodeModel::CiscoCat8000v
            | NodeModel::CiscoCat9000v
            | NodeModel::CiscoAsav
            | NodeModel::CiscoNexus9300v
            | NodeModel::CiscoIosxrv9000
            | NodeModel::CiscoFtdv
            | NodeModel::JuniperVrouter
            | NodeModel::JuniperVswitch
            | NodeModel::PaloaltoPanos => vec![Ztp::Cdrom],
            NodeModel::CiscoIosv
            | NodeModel::CiscoIosvl2
            | NodeModel::CiscoIse
            | NodeModel::MikrotikChr => vec![Ztp::Disk],
            NodeModel::CumulusLinux => vec![Ztp::Usb],
            NodeModel::SonicLinux => vec![Ztp::Http],
            NodeModel::FlatcarLinux => vec![Ztp::Ignition],
            NodeModel::TalosLinux => vec![Ztp::NoCloud],
            NodeModel::AristaCeos
            | NodeModel::NokiaSrlinux
            | NodeModel::JuniperCevolved
            | NodeModel::FrrLinux => vec![Ztp::Volume],
            NodeModel::GitlabCe
            | NodeModel::ForgejoForge
            | NodeModel::SurrealDb
            | NodeModel::MysqlDb
            | NodeModel::MongoDb
            | NodeModel::PostgresqlDb
            | NodeModel::NatsBus
            | NodeModel::HashicorpVault
            | NodeModel::KeycloakServer
            | NodeModel::GenericContainer
            | NodeModel::GenericUnikernel
            | NodeModel::UnikraftUnikernel
            | NodeModel::NanosUnikernel => Vec::new(),
        }
    }

    #[test]
    fn test_ztp_methods_of_every_model_match_the_generators() {
        for model in NodeModel::iter() {
            assert_eq!(
                model.ztp_methods(),
                expected_ztp_methods(model),
                "methods of {model}"
            );
        }
    }

    #[test]
    fn test_no_model_lists_disabled_as_a_ztp_method() {
        for model in NodeModel::iter() {
            assert!(
                !model.ztp_methods().contains(&Ztp::Disabled),
                "{model} lists disabled"
            );
        }
    }

    #[test]
    fn test_every_models_default_ztp_is_disabled_or_a_listed_method() {
        for model in NodeModel::iter() {
            let ztp = NodeConfig::get_model(model).ztp.method;
            assert!(
                ztp == Ztp::Disabled || model.ztp_methods().contains(&ztp),
                "{model} defaults to {ztp}, which it does not list"
            );
        }
    }

    #[test]
    fn test_container_and_unikernel_default_ztp() {
        let mounted = [
            NodeModel::AristaCeos,
            NodeModel::NokiaSrlinux,
            NodeModel::JuniperCevolved,
            NodeModel::FrrLinux,
        ];
        for model in NodeModel::iter() {
            let config = NodeConfig::get_model(model);
            let expected = match config.kind {
                NodeKind::Container if mounted.contains(&model) => Ztp::Volume,
                NodeKind::Container | NodeKind::Unikernel => Ztp::Disabled,
                NodeKind::VirtualMachine => continue,
            };
            assert_eq!(config.ztp.method, expected, "default of {model}");
        }
    }

    /// Every model's JSON nests its settings by group, with exactly these fields in each, and
    /// reads back to the same JSON.
    #[test]
    fn test_node_config_json_nests_the_settings_by_group() {
        fn keys(value: &serde_json::Value) -> Vec<&str> {
            let mut keys: Vec<&str> = value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            keys
        }
        let groups = [
            ("cpu", vec!["architecture", "count", "model", "vmx"]),
            ("machine", vec!["bios", "kind"]),
            ("boot_disk", vec!["bus"]),
            ("cdrom", vec!["bus", "image"]),
            (
                "interfaces",
                vec![
                    "data_count",
                    "first_index",
                    "kind",
                    "management",
                    "mtu",
                    "prefix",
                    "reserved_count",
                ],
            ),
            (
                "ztp",
                vec!["method", "password", "password_auth", "username"],
            ),
        ];
        let mut top = vec![
            "id",
            "model",
            "version",
            "repo",
            "os_variant",
            "kind",
            "default",
            "memory",
            "unikernel",
        ];
        top.extend(groups.iter().map(|(group, _)| *group));
        top.sort_unstable();
        for model in NodeModel::iter() {
            let json = serde_json::to_value(NodeConfig::get_model(model)).unwrap();
            assert_eq!(keys(&json), top, "{model}");
            for (group, fields) in &groups {
                assert_eq!(keys(&json[group]), *fields, "{model} {group}");
            }
            match model.kind() {
                NodeKind::Unikernel => {
                    assert_eq!(keys(&json["unikernel"]), ["boot_mode"], "{model}");
                }
                _ => assert!(json["unikernel"].is_null(), "{model}"),
            }
            let back: NodeConfig = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(back).unwrap(), json, "{model}");
        }
    }

    #[test]
    fn test_node_config_json_refuses_the_old_flat_names() {
        let mut json = serde_json::to_value(NodeConfig::ubuntu_linux()).unwrap();
        json["cpu_count"] = serde_json::json!(2);
        let error = serde_json::from_value::<NodeConfig>(json)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown field `cpu_count`"), "{error}");
    }

    /// Every model's built-in image, serialised in field order, is byte-for-byte the checked-in
    /// fixture. Any change to a model, or to a base it is built from, shows up as a fixture diff
    /// naming every model it touched.
    #[test]
    fn test_every_model_config_matches_the_snapshot() {
        let models: Vec<NodeConfig> = NodeModel::iter().map(NodeConfig::get_model).collect();
        let json = format!("{}\n", serde_json::to_string_pretty(&models).unwrap());
        assert_eq!(json, include_str!("node_models.json"));
    }
}
