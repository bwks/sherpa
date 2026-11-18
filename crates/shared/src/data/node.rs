use std::ffi::OsStr;
use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};
use clap::ValueEnum;
use clap::builder::{PossibleValue, TypedValueParser, ValueParserFactory};
use clap::error::ErrorKind;
use schemars::JsonSchema;
use serde::de::{self, Deserializer};
use serde_derive::{Deserialize, Serialize};

use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, IntoStaticStr};

use super::cpu::CpuModels;
use super::disk::DiskBuses;
use super::interface::MgmtInterfaces;
use crate::konst::{
    CONTAINER_ARISTA_EOS_REPO, CONTAINER_CISCO_IOL_REPO, CONTAINER_CISCO_IOLL2_REPO,
    CONTAINER_FORGEJO_REPO, CONTAINER_FRR_REPO, CONTAINER_GITLAB_CE_REPO,
    CONTAINER_HASHICORP_VAULT_REPO, CONTAINER_JUNIPER_EVOLVED_REPO, CONTAINER_KEYCLOAK_REPO,
    CONTAINER_MONGO_DB_REPO, CONTAINER_NATS_REPO, CONTAINER_NOKIA_SRLINUX_REPO,
    CONTAINER_SONIC_LINUX_REPO, CONTAINER_SURREAL_DB_REPO, MTU_JUMBO_INT, MTU_JUMBO_NET, MTU_STD,
    SONIC_PORT_COUNT,
};

use super::RecordId;
use super::kind_settings::{self, CONTAINER_GROUP, UNIKERNEL_GROUP, VM_GROUP};
use super::model_kind::ModelKind;
use super::node_base;

/// A node model, the name a manifest, a command and the API give it. It is read only through
/// `FromStr`, from JSON and TOML, from a command line and from the database alike, so a name
/// removed in 0.7.0 is refused everywhere with the same message naming its replacement.
#[derive(
    Default, PartialEq, Eq, Hash, Clone, Copy, Debug, Serialize, EnumIter, IntoStaticStr, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum NodeModel {
    #[default]
    // Arista
    AristaEos,

    // Aruba
    ArubaAoscx,

    // Cisco
    CiscoAsa,
    CiscoCsr1000,
    CiscoCat8000,
    CiscoCat9000,
    CiscoIosxr9000,
    CiscoNexus9300,
    CiscoIos,
    CiscoIosl2,
    CiscoIol,
    CiscoIoll2,
    CiscoIse,
    CiscoFtd,

    // Juniper
    JuniperRouter,
    JuniperSwitch,
    JuniperEvolved,
    JuniperSrxv3,

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
    Generic,

    // Unikernels
    UnikraftUnikernel,
    NanosUnikernel,
}
impl fmt::Display for NodeModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Arista
            NodeModel::AristaEos => write!(f, "arista_eos"),

            // Aruba
            NodeModel::ArubaAoscx => write!(f, "aruba_aoscx"),

            // Cisco
            NodeModel::CiscoAsa => write!(f, "cisco_asa"),
            NodeModel::CiscoCsr1000 => write!(f, "cisco_csr1000"),
            NodeModel::CiscoCat8000 => write!(f, "cisco_cat8000"),
            NodeModel::CiscoCat9000 => write!(f, "cisco_cat9000"),
            NodeModel::CiscoIosxr9000 => write!(f, "cisco_iosxr9000"),
            NodeModel::CiscoNexus9300 => write!(f, "cisco_nexus9300"),
            NodeModel::CiscoIos => write!(f, "cisco_ios"),
            NodeModel::CiscoIosl2 => write!(f, "cisco_iosl2"),
            NodeModel::CiscoIol => write!(f, "cisco_iol"),
            NodeModel::CiscoIoll2 => write!(f, "cisco_ioll2"),
            NodeModel::CiscoIse => write!(f, "cisco_ise"),
            NodeModel::CiscoFtd => write!(f, "cisco_ftd"),

            // Juniper
            NodeModel::JuniperRouter => write!(f, "juniper_router"),
            NodeModel::JuniperSwitch => write!(f, "juniper_switch"),
            NodeModel::JuniperEvolved => write!(f, "juniper_evolved"),
            NodeModel::JuniperSrxv3 => write!(f, "juniper_srxv3"),

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
            NodeModel::Generic => write!(f, "generic"),

            // Unikernels
            NodeModel::UnikraftUnikernel => write!(f, "unikraft_unikernel"),
            NodeModel::NanosUnikernel => write!(f, "nanos_unikernel"),
        }
    }
}
impl FromStr for NodeModel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            // Arista
            "arista_eos" => Ok(NodeModel::AristaEos),

            // Aruba
            "aruba_aoscx" => Ok(NodeModel::ArubaAoscx),

            // Cisco
            "cisco_asa" => Ok(NodeModel::CiscoAsa),
            "cisco_csr1000" => Ok(NodeModel::CiscoCsr1000),
            "cisco_cat8000" => Ok(NodeModel::CiscoCat8000),
            "cisco_cat9000" => Ok(NodeModel::CiscoCat9000),
            "cisco_iosxr9000" => Ok(NodeModel::CiscoIosxr9000),
            "cisco_nexus9300" => Ok(NodeModel::CiscoNexus9300),
            "cisco_ios" => Ok(NodeModel::CiscoIos),
            "cisco_iosl2" => Ok(NodeModel::CiscoIosl2),
            "cisco_iol" => Ok(NodeModel::CiscoIol),
            "cisco_ioll2" => Ok(NodeModel::CiscoIoll2),
            "cisco_ise" => Ok(NodeModel::CiscoIse),
            "cisco_ftd" => Ok(NodeModel::CiscoFtd),

            // Juniper
            "juniper_router" => Ok(NodeModel::JuniperRouter),
            "juniper_switch" => Ok(NodeModel::JuniperSwitch),
            "juniper_evolved" => Ok(NodeModel::JuniperEvolved),
            "juniper_srxv3" => Ok(NodeModel::JuniperSrxv3),

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
            "generic" => Ok(NodeModel::Generic),

            // Unikernels
            "unikraft_unikernel" => Ok(NodeModel::UnikraftUnikernel),
            "nanos_unikernel" => Ok(NodeModel::NanosUnikernel),

            _ => match REMOVED_MODELS.iter().find(|(name, _, _)| *name == s) {
                Some((_, model, kind)) => Err(format!(
                    "Unknown node model '{s}': it was renamed in 0.7.0. \
                     Use model = \"{model}\" with kind = \"{kind}\"."
                )),
                None => Err(format!("Unknown node model: {s}")),
            },
        }
    }
}
impl<'de> serde::Deserialize<'de> for NodeModel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        NodeModel::from_str(&name).map_err(de::Error::custom)
    }
}

/// Reads a [`NodeModel`] from a command line through its `FromStr`, so an old name gets the
/// message naming its replacement, and lists every model as the argument's possible values.
#[derive(Clone, Copy, Debug)]
pub struct NodeModelParser;

impl TypedValueParser for NodeModelParser {
    type Value = NodeModel;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<NodeModel, clap::Error> {
        let name = value.to_string_lossy();
        NodeModel::from_str(&name).map_err(|message| {
            let argument = arg.map_or_else(String::new, |arg| format!(" for '{arg}'"));
            clap::Error::raw(
                ErrorKind::InvalidValue,
                format!("invalid value '{name}'{argument}: {message}\n"),
            )
            .with_cmd(cmd)
        })
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(NodeModel::iter().map(|model| {
            let name: &'static str = model.into();
            PossibleValue::new(name)
        })))
    }
}

impl ValueParserFactory for NodeModel {
    type Parser = NodeModelParser;

    fn value_parser() -> NodeModelParser {
        NodeModelParser
    }
}

/// The model names removed in 0.7.0, each with the model and kind that replace it. They are
/// known only to refuse an old name with a message naming its replacement, and to move the
/// image folders named after them. To be deleted in 0.8.0.
pub const REMOVED_MODELS: [(&str, NodeModel, NodeKind); 19] = [
    (
        "arista_veos",
        NodeModel::AristaEos,
        NodeKind::VirtualMachine,
    ),
    ("arista_ceos", NodeModel::AristaEos, NodeKind::Container),
    ("cisco_asav", NodeModel::CiscoAsa, NodeKind::VirtualMachine),
    ("cisco_ftdv", NodeModel::CiscoFtd, NodeKind::VirtualMachine),
    (
        "cisco_iosxrv9000",
        NodeModel::CiscoIosxr9000,
        NodeKind::VirtualMachine,
    ),
    ("cisco_iosv", NodeModel::CiscoIos, NodeKind::VirtualMachine),
    (
        "cisco_iosvl2",
        NodeModel::CiscoIosl2,
        NodeKind::VirtualMachine,
    ),
    (
        "cisco_csr1000v",
        NodeModel::CiscoCsr1000,
        NodeKind::VirtualMachine,
    ),
    (
        "cisco_cat8000v",
        NodeModel::CiscoCat8000,
        NodeKind::VirtualMachine,
    ),
    (
        "cisco_cat9000v",
        NodeModel::CiscoCat9000,
        NodeKind::VirtualMachine,
    ),
    (
        "cisco_nexus9300v",
        NodeModel::CiscoNexus9300,
        NodeKind::VirtualMachine,
    ),
    (
        "juniper_vrouter",
        NodeModel::JuniperRouter,
        NodeKind::VirtualMachine,
    ),
    (
        "juniper_vswitch",
        NodeModel::JuniperSwitch,
        NodeKind::VirtualMachine,
    ),
    (
        "juniper_vevolved",
        NodeModel::JuniperEvolved,
        NodeKind::VirtualMachine,
    ),
    (
        "juniper_cevolved",
        NodeModel::JuniperEvolved,
        NodeKind::Container,
    ),
    (
        "juniper_vsrxv3",
        NodeModel::JuniperSrxv3,
        NodeKind::VirtualMachine,
    ),
    ("generic_vm", NodeModel::Generic, NodeKind::VirtualMachine),
    ("generic_container", NodeModel::Generic, NodeKind::Container),
    ("generic_unikernel", NodeModel::Generic, NodeKind::Unikernel),
];

impl NodeModel {
    #[must_use]
    pub fn to_vec() -> Vec<NodeModel> {
        NodeModel::iter().collect()
    }

    /// The kinds this model takes, in a fixed order. The first is the model's default, the
    /// kind a node gets when its manifest sets no `kind`, and it never changes; `generic` has
    /// no default (see [`NodeModel::default_kind`]). Every model is named, so a new one does not
    /// compile until it has an entry, and every entry lists at least one kind.
    #[must_use]
    pub fn kinds(self) -> &'static [NodeKind] {
        match self {
            NodeModel::AristaEos | NodeModel::JuniperEvolved | NodeModel::SonicLinux => {
                &[NodeKind::VirtualMachine, NodeKind::Container]
            }
            NodeModel::Generic => &[
                NodeKind::VirtualMachine,
                NodeKind::Container,
                NodeKind::Unikernel,
            ],
            NodeModel::ArubaAoscx
            | NodeModel::CiscoAsa
            | NodeModel::CiscoCsr1000
            | NodeModel::CiscoCat8000
            | NodeModel::CiscoCat9000
            | NodeModel::CiscoIosxr9000
            | NodeModel::CiscoNexus9300
            | NodeModel::CiscoIos
            | NodeModel::CiscoIosl2
            | NodeModel::CiscoIse
            | NodeModel::CiscoFtd
            | NodeModel::JuniperRouter
            | NodeModel::JuniperSwitch
            | NodeModel::JuniperSrxv3
            | NodeModel::PaloaltoPanos
            | NodeModel::MikrotikChr
            | NodeModel::AlmaLinux
            | NodeModel::RockyLinux
            | NodeModel::AlpineLinux
            | NodeModel::CumulusLinux
            | NodeModel::CentosLinux
            | NodeModel::DevboxLinux
            | NodeModel::FedoraLinux
            | NodeModel::RedhatLinux
            | NodeModel::OpensuseLinux
            | NodeModel::SuseLinux
            | NodeModel::UbuntuLinux
            | NodeModel::OmarchyLinux
            | NodeModel::NixosLinux
            | NodeModel::KaliLinux
            | NodeModel::FlatcarLinux
            | NodeModel::TalosLinux
            | NodeModel::DevboxWindows
            | NodeModel::WindowsServer
            | NodeModel::FreeBsd
            | NodeModel::OpenBsd
            | NodeModel::InfrahubServer
            | NodeModel::JenkinsServer
            | NodeModel::NautobotServer
            | NodeModel::NetboxServer
            | NodeModel::SignozServer
            | NodeModel::VirtServer => &[NodeKind::VirtualMachine],
            NodeModel::CiscoIol
            | NodeModel::CiscoIoll2
            | NodeModel::NokiaSrlinux
            | NodeModel::FrrLinux
            | NodeModel::ForgejoForge
            | NodeModel::GitlabCe
            | NodeModel::NatsBus
            | NodeModel::SurrealDb
            | NodeModel::MysqlDb
            | NodeModel::MongoDb
            | NodeModel::PostgresqlDb
            | NodeModel::HashicorpVault
            | NodeModel::KeycloakServer => &[NodeKind::Container],
            NodeModel::UnikraftUnikernel | NodeModel::NanosUnikernel => &[NodeKind::Unikernel],
        }
    }

    /// The kind a node of this model gets when its manifest or request sets none: the first of
    /// [`NodeModel::kinds`]. `None` for `generic`, which runs as any kind and so must be told
    /// which.
    #[must_use]
    pub fn default_kind(self) -> Option<NodeKind> {
        match self {
            NodeModel::Generic => None,
            model => Some(model.kinds()[0]),
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
    Clone,
    Copy,
    Debug,
    Deserialize,
    Default,
    Serialize,
    PartialEq,
    Eq,
    Hash,
    ValueEnum,
    EnumIter,
    JsonSchema,
)]
#[clap(rename_all = "snake_case")]
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

/// Zero touch provisioning for a node image: off or the method Sherpa provisions its nodes by.
/// It denies unknown fields, so a `ztp` group that still carries `username`, `password` or
/// `password_auth`, removed in 0.7.0, is refused with serde's error naming the field.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZtpSettings {
    /// `disabled`, or how the image's nodes are provisioned at boot.
    pub method: Ztp,
}

/// A node image's virtual CPUs, for every kind of node: how many. The CPU a virtual machine or
/// unikernel emulates is in its kind's group.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CpuSettings {
    /// The number of vCPUs.
    pub count: u8,
}

/// The CPU a virtual machine emulates.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VmCpuSettings {
    pub architecture: CpuArchitecture,
    pub model: CpuModels,
    /// Whether the guest sees the host's hardware virtualisation, to run VMs of its own.
    pub vmx: bool,
}

/// The CPU a unikernel emulates.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnikernelCpuSettings {
    pub architecture: CpuArchitecture,
    pub model: CpuModels,
}

/// The machine a virtual machine runs on: the QEMU machine type and the firmware.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MachineSettings {
    /// The QEMU machine type.
    pub kind: MachineType,
    pub bios: BiosTypes,
}

/// The machine a unikernel runs on: the QEMU machine type. A unikernel has no firmware setting.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnikernelMachineSettings {
    /// The QEMU machine type.
    pub kind: MachineType,
}

/// The disk a virtual machine boots from.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BootDiskSettings {
    /// The bus the boot disk attaches to.
    pub bus: DiskBuses,
}

/// The CD-ROM a virtual machine boots with, if any, and the bus CD-ROMs attach to.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdromSettings {
    /// The file name of a CD-ROM image shipped with the node image.
    pub image: Option<String>,
    pub bus: DiskBuses,
}

/// A node image's network interfaces, for every kind of node: how many, and which one is the
/// management interface. The device a virtual machine or unikernel gives them is in its kind's
/// group.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InterfaceSettings {
    /// The number of data interfaces, not counting the management interface.
    pub data_count: u8,
    pub management: MgmtInterfaces,
    /// Interfaces the node holds back from the data interfaces a lab can link.
    pub reserved_count: u8,
}

/// The network device a virtual machine or unikernel gives each interface, and its MTU.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InterfaceDeviceSettings {
    /// The interface device QEMU emulates.
    pub kind: InterfaceType,
    pub mtu: u16,
}

/// `vm`: the settings only a virtual machine image takes.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VmSettings {
    /// The guest OS family, which decides how a cloud-init guest's account is written.
    pub os_variant: OsVariant,
    pub cpu: VmCpuSettings,
    pub machine: MachineSettings,
    pub boot_disk: BootDiskSettings,
    pub cdrom: CdromSettings,
    pub interfaces: InterfaceDeviceSettings,
    pub ztp: ZtpSettings,
}

/// `container`: the settings only a container image takes.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContainerSettings {
    /// The Docker repo the image was pulled or imported as; the version is its tag.
    pub repo: Option<String>,
    pub ztp: ZtpSettings,
}

/// `unikernel`: the settings only a unikernel image takes.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnikernelSettings {
    pub boot_mode: UnikernelBootMode,
    pub cpu: UnikernelCpuSettings,
    pub machine: UnikernelMachineSettings,
    pub interfaces: InterfaceDeviceSettings,
}

/// The settings of one kind of node image. An image holds exactly one, and its kind is which.
#[derive(Clone, Debug, PartialEq)]
pub enum KindSettings {
    VirtualMachine(VmSettings),
    Container(ContainerSettings),
    Unikernel(UnikernelSettings),
}

impl KindSettings {
    /// The kind of node these settings are for.
    #[must_use]
    pub fn kind(&self) -> NodeKind {
        match self {
            KindSettings::VirtualMachine(_) => NodeKind::VirtualMachine,
            KindSettings::Container(_) => NodeKind::Container,
            KindSettings::Unikernel(_) => NodeKind::Unikernel,
        }
    }
}

/// A node image: the model and version, and how Sherpa builds and boots its nodes. It holds
/// the groups every kind takes and exactly one kind's group, `vm`, `container` or
/// `unikernel`, so an image cannot hold another kind's settings. In JSON the kind is the
/// `kind` field and the group sits beside it; a group that does not match `kind`, or a second
/// group, is refused.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(try_from = "NodeConfigFields", into = "NodeConfigFields")]
pub struct NodeConfig {
    pub id: Option<RecordId>,
    pub model: NodeModel,
    pub version: String,
    /// Whether this version is the default of the model's kind.
    pub default: bool,
    /// The guest's memory, in MiB.
    pub memory: u16,
    pub cpu: CpuSettings,
    pub interfaces: InterfaceSettings,
    /// The settings of the image's kind.
    pub settings: KindSettings,
}

/// A [`NodeConfig`] as JSON carries it: the kind, and one optional group per kind. Reading
/// it checks the groups against the kind with the per-kind model in `kind_settings`.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NodeConfigFields {
    #[schemars(skip)]
    id: Option<RecordId>,
    model: NodeModel,
    version: String,
    kind: NodeKind,
    /// Whether this version is the default of the model's kind.
    default: bool,
    /// The guest's memory, in MiB.
    memory: u16,
    cpu: CpuSettings,
    interfaces: InterfaceSettings,
    /// A virtual machine image's settings; only with `kind = "virtual_machine"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vm: Option<VmSettings>,
    /// A container image's settings; only with `kind = "container"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    container: Option<ContainerSettings>,
    /// A unikernel image's settings; only with `kind = "unikernel"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unikernel: Option<UnikernelSettings>,
}

impl TryFrom<NodeConfigFields> for NodeConfig {
    type Error = anyhow::Error;

    fn try_from(fields: NodeConfigFields) -> Result<Self> {
        let groups = [
            (VM_GROUP, fields.vm.is_some(), kind_settings::VM),
            (
                CONTAINER_GROUP,
                fields.container.is_some(),
                kind_settings::CONTAINER,
            ),
            (
                UNIKERNEL_GROUP,
                fields.unikernel.is_some(),
                kind_settings::UNIKERNEL,
            ),
        ];
        if let Some((group, ..)) = kind_settings::first_not_taken(fields.kind, &groups) {
            bail!(
                "Node image '{}' is a {} image, which does not take the `{group}` group",
                fields.model,
                fields.kind
            );
        }
        let settings = match (fields.kind, fields.vm, fields.container, fields.unikernel) {
            (NodeKind::VirtualMachine, Some(vm), _, _) => KindSettings::VirtualMachine(vm),
            (NodeKind::Container, _, Some(container), _) => KindSettings::Container(container),
            (NodeKind::Unikernel, _, _, Some(unikernel)) => KindSettings::Unikernel(unikernel),
            (kind, ..) => bail!(
                "Node image '{}' is a {kind} image and needs its `{}` group",
                fields.model,
                kind_settings::group_of(kind)
            ),
        };
        Ok(Self {
            id: fields.id,
            model: fields.model,
            version: fields.version,
            default: fields.default,
            memory: fields.memory,
            cpu: fields.cpu,
            interfaces: fields.interfaces,
            settings,
        })
    }
}

impl From<NodeConfig> for NodeConfigFields {
    fn from(config: NodeConfig) -> Self {
        let kind = config.kind();
        let (vm, container, unikernel) = match config.settings {
            KindSettings::VirtualMachine(vm) => (Some(vm), None, None),
            KindSettings::Container(container) => (None, Some(container), None),
            KindSettings::Unikernel(unikernel) => (None, None, Some(unikernel)),
        };
        Self {
            id: config.id,
            model: config.model,
            version: config.version,
            kind,
            default: config.default,
            memory: config.memory,
            cpu: config.cpu,
            interfaces: config.interfaces,
            vm,
            container,
            unikernel,
        }
    }
}

impl NodeConfig {
    /// The kind of node this image is for: which group it holds.
    #[must_use]
    pub fn kind(&self) -> NodeKind {
        self.settings.kind()
    }

    /// The model and kind of this image.
    ///
    /// # Errors
    ///
    /// Fails when the image's model does not take its kind, as a record edited by hand could.
    pub fn model_kind(&self) -> Result<ModelKind> {
        ModelKind::new(self.model, Some(self.kind()))
    }

    /// The image's ZTP setting: its group's, and `disabled` for a unikernel, which has none.
    #[must_use]
    pub fn ztp(&self) -> Ztp {
        match &self.settings {
            KindSettings::VirtualMachine(vm) => vm.ztp.method,
            KindSettings::Container(container) => container.ztp.method,
            KindSettings::Unikernel(_) => Ztp::Disabled,
        }
    }

    /// The image's `vm` group, if it is a virtual machine image.
    #[must_use]
    pub fn vm(&self) -> Option<&VmSettings> {
        match &self.settings {
            KindSettings::VirtualMachine(vm) => Some(vm),
            KindSettings::Container(_) | KindSettings::Unikernel(_) => None,
        }
    }

    /// The image's `container` group, if it is a container image.
    #[must_use]
    pub fn container(&self) -> Option<&ContainerSettings> {
        match &self.settings {
            KindSettings::Container(container) => Some(container),
            KindSettings::VirtualMachine(_) | KindSettings::Unikernel(_) => None,
        }
    }

    /// The image's `unikernel` group, if it is a unikernel image.
    #[must_use]
    pub fn unikernel(&self) -> Option<&UnikernelSettings> {
        match &self.settings {
            KindSettings::Unikernel(unikernel) => Some(unikernel),
            KindSettings::VirtualMachine(_) | KindSettings::Container(_) => None,
        }
    }

    /// The network device of a virtual machine or unikernel image; `None` for a container,
    /// whose interfaces Docker makes.
    #[must_use]
    pub fn interface_device(&self) -> Option<&InterfaceDeviceSettings> {
        match &self.settings {
            KindSettings::VirtualMachine(vm) => Some(&vm.interfaces),
            KindSettings::Unikernel(unikernel) => Some(&unikernel.interfaces),
            KindSettings::Container(_) => None,
        }
    }

    /// The repo a container node runs, for the node's version as pulled (`self`): the
    /// manifest's `image`, or else the repo its version was pulled from or imported as; `None`
    /// if it has neither, or if the image is not a container's. A model with a built-in repo is
    /// treated the same way: its built-in repo is only where its records start.
    #[must_use]
    pub fn container_repo(&self, manifest_image: Option<&str>) -> Option<String> {
        manifest_image.map(str::to_string).or_else(|| {
            self.container()
                .and_then(|container| container.repo.clone())
        })
    }
    #[must_use]
    pub fn arista_eos_vm() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Eos,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::image(
                NodeModel::AristaEos,
                2048,
                InterfaceSettings {
                    data_count: 52,
                    management: MgmtInterfaces::Management1,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    boot_disk: BootDiskSettings {
                        bus: DiskBuses::Sata,
                    },
                    cdrom: CdromSettings {
                        image: Some("aboot.iso".to_owned()),
                        bus: DiskBuses::Ide,
                    },
                    ztp: ZtpSettings { method: Ztp::Tftp },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn arista_eos_container() -> NodeConfig {
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::network_os_container(
                NodeModel::AristaEos,
                4096,
                CONTAINER_ARISTA_EOS_REPO,
                InterfaceSettings {
                    data_count: 52,
                    management: MgmtInterfaces::Eth0,
                    reserved_count: 0,
                },
            )
        }
    }
    #[must_use]
    pub fn aruba_aoscx() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Aos,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::image(
                NodeModel::ArubaAoscx,
                4096,
                InterfaceSettings {
                    data_count: 52,
                    management: MgmtInterfaces::Mgmt,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    machine: MachineSettings {
                        kind: MachineType::Q35,
                        ..vm.machine
                    },
                    ztp: ZtpSettings { method: Ztp::Tftp },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn cisco_asa() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Asa,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
            },
        );
        node_base::image(
            NodeModel::CiscoAsa,
            2048,
            InterfaceSettings {
                data_count: 8,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
            KindSettings::VirtualMachine(VmSettings {
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                cdrom: CdromSettings {
                    bus: DiskBuses::Ide,
                    ..vm.cdrom
                },
                ..vm
            }),
        )
    }
    #[must_use]
    pub fn cisco_csr1000() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Iosxe,
            InterfaceDeviceSettings {
                kind: InterfaceType::Vmxnet3,
                mtu: MTU_JUMBO_INT,
            },
        );
        node_base::image(
            NodeModel::CiscoCsr1000,
            3072,
            InterfaceSettings {
                data_count: 15,
                management: MgmtInterfaces::GigabitEthernet1,
                reserved_count: 0,
            },
            KindSettings::VirtualMachine(VmSettings {
                machine: MachineSettings {
                    kind: MachineType::Q35,
                    ..vm.machine
                },
                ..vm
            }),
        )
    }
    #[must_use]
    pub fn cisco_cat8000() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Iosxe,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
            },
        );
        node_base::image(
            NodeModel::CiscoCat8000,
            4096,
            InterfaceSettings {
                data_count: 15,
                management: MgmtInterfaces::GigabitEthernet1,
                reserved_count: 0,
            },
            KindSettings::VirtualMachine(VmSettings {
                machine: MachineSettings {
                    kind: MachineType::Q35,
                    ..vm.machine
                },
                ..vm
            }),
        )
    }
    #[must_use]
    pub fn cisco_cat9000() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Iosxe,
            InterfaceDeviceSettings {
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 4 },
            ..node_base::image(
                NodeModel::CiscoCat9000,
                18432,
                InterfaceSettings {
                    data_count: 8,
                    management: MgmtInterfaces::GigabitEthernet0_0,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    machine: MachineSettings {
                        kind: MachineType::Q35,
                        ..vm.machine
                    },
                    boot_disk: BootDiskSettings {
                        bus: DiskBuses::Sata,
                    },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn cisco_iosxr9000() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Iosxr,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 4 },
            ..node_base::image(
                NodeModel::CiscoIosxr9000,
                20480,
                InterfaceSettings {
                    data_count: 31,
                    management: MgmtInterfaces::MgmtEth0Rp0Cpu0_0,
                    reserved_count: 2,
                },
                KindSettings::VirtualMachine(VmSettings {
                    machine: MachineSettings {
                        bios: BiosTypes::Uefi,
                        ..vm.machine
                    },
                    boot_disk: BootDiskSettings {
                        bus: DiskBuses::Sata,
                    },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn cisco_nexus9300() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Nxos,
            InterfaceDeviceSettings {
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::image(
                NodeModel::CiscoNexus9300,
                12288,
                InterfaceSettings {
                    data_count: 64,
                    management: MgmtInterfaces::Mgmt0,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    machine: MachineSettings {
                        bios: BiosTypes::Uefi,
                        ..vm.machine
                    },
                    boot_disk: BootDiskSettings {
                        bus: DiskBuses::Sata,
                    },
                    cdrom: CdromSettings {
                        bus: DiskBuses::Ide,
                        ..vm.cdrom
                    },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn cisco_ios() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Ios,
            InterfaceDeviceSettings {
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
            },
        );
        node_base::image(
            NodeModel::CiscoIos,
            768,
            InterfaceSettings {
                data_count: 15,
                management: MgmtInterfaces::GigabitEthernet0_0,
                reserved_count: 0,
            },
            KindSettings::VirtualMachine(VmSettings {
                cdrom: CdromSettings {
                    bus: DiskBuses::Virtio,
                    ..vm.cdrom
                },
                ztp: ZtpSettings { method: Ztp::Disk },
                ..vm
            }),
        )
    }
    #[must_use]
    pub fn cisco_iosl2() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Ios,
            InterfaceDeviceSettings {
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
            },
        );
        node_base::image(
            NodeModel::CiscoIosl2,
            1024,
            InterfaceSettings {
                // IOSvL2 crashes with more than 16 NICs: GigabitEthernet0/0 for management plus 15 data.
                data_count: 15,
                management: MgmtInterfaces::GigabitEthernet0_0,
                reserved_count: 0,
            },
            KindSettings::VirtualMachine(VmSettings {
                cdrom: CdromSettings {
                    bus: DiskBuses::Virtio,
                    ..vm.cdrom
                },
                ztp: ZtpSettings { method: Ztp::Disk },
                ..vm
            }),
        )
    }
    /// Cisco IOL XE, from the user's CML image rebuilt by docker-cisco-iol. 16 Ethernet ports in
    /// slots of 4: `Ethernet0/0` for management and 15 data ports.
    #[must_use]
    pub fn cisco_iol() -> NodeConfig {
        node_base::network_os_container(
            NodeModel::CiscoIol,
            768,
            CONTAINER_CISCO_IOL_REPO,
            cisco_iol_interfaces(),
        )
    }
    /// Cisco IOL XE L2, the switch image; as `cisco_iol`.
    #[must_use]
    pub fn cisco_ioll2() -> NodeConfig {
        node_base::network_os_container(
            NodeModel::CiscoIoll2,
            768,
            CONTAINER_CISCO_IOLL2_REPO,
            cisco_iol_interfaces(),
        )
    }
    #[must_use]
    pub fn cisco_ise() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Ise,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 4 },
            ..node_base::image(
                NodeModel::CiscoIse,
                16384,
                InterfaceSettings {
                    data_count: 1,
                    management: MgmtInterfaces::Eth0,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    ztp: ZtpSettings { method: Ztp::Disk },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn cisco_ftd() -> NodeConfig {
        NodeConfig {
            cpu: CpuSettings { count: 4 },
            ..node_base::image(
                NodeModel::CiscoFtd,
                8192,
                InterfaceSettings {
                    data_count: 8,
                    management: MgmtInterfaces::Management0_0,
                    reserved_count: 1,
                },
                KindSettings::VirtualMachine(node_base::network_os_vm_settings(
                    OsVariant::Fxos,
                    InterfaceDeviceSettings {
                        kind: InterfaceType::Virtio,
                        mtu: MTU_JUMBO_INT,
                    },
                )),
            )
        }
    }
    #[must_use]
    pub fn juniper_router() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Junos,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 4 },
            ..node_base::image(
                NodeModel::JuniperRouter,
                5120,
                InterfaceSettings {
                    data_count: 10,
                    management: MgmtInterfaces::Fxp0,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    cpu: VmCpuSettings {
                        model: CpuModels::IvyBridge,
                        vmx: true,
                        ..vm.cpu
                    },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn juniper_switch() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Junos,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 4 },
            ..node_base::image(
                NodeModel::JuniperSwitch,
                5120,
                InterfaceSettings {
                    data_count: 10,
                    management: MgmtInterfaces::Fxp0,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    cpu: VmCpuSettings {
                        model: CpuModels::IvyBridge,
                        vmx: true,
                        ..vm.cpu
                    },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn juniper_evolved_vm() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Junos,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 4 },
            ..node_base::image(
                NodeModel::JuniperEvolved,
                8192,
                InterfaceSettings {
                    data_count: 12,
                    management: MgmtInterfaces::Re0Mgmt0,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    cpu: VmCpuSettings {
                        model: CpuModels::IvyBridge,
                        vmx: true,
                        ..vm.cpu
                    },
                    machine: MachineSettings {
                        bios: BiosTypes::Uefi,
                        ..vm.machine
                    },
                    ztp: ZtpSettings { method: Ztp::Tftp },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn juniper_srxv3() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Junos,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::image(
                NodeModel::JuniperSrxv3,
                4096,
                InterfaceSettings {
                    data_count: 8,
                    management: MgmtInterfaces::Fxp0,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    cpu: VmCpuSettings {
                        model: CpuModels::SandyBridge,
                        vmx: true,
                        ..vm.cpu
                    },
                    machine: MachineSettings {
                        kind: MachineType::PcI440Fx8_0,
                        ..vm.machine
                    },
                    cdrom: CdromSettings {
                        bus: DiskBuses::Ide,
                        ..vm.cdrom
                    },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn alma_linux() -> NodeConfig {
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::AlmaLinux)
        }
    }
    #[must_use]
    pub fn rocky_linux() -> NodeConfig {
        let vm = node_base::cloud_init_vm_settings();
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                machine: MachineSettings {
                    bios: BiosTypes::Uefi,
                    ..vm.machine
                },
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                ..vm
            }),
            ..node_base::cloud_init_vm(NodeModel::RockyLinux)
        }
    }
    #[must_use]
    pub fn alpine_linux() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::AlpineLinux);
        NodeConfig {
            interfaces: InterfaceSettings {
                data_count: 2,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn cumulus_linux() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Nvue,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::image(
                NodeModel::CumulusLinux,
                2048,
                InterfaceSettings {
                    data_count: 52,
                    management: MgmtInterfaces::Eth0,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    machine: MachineSettings {
                        kind: MachineType::Q35,
                        ..vm.machine
                    },
                    boot_disk: BootDiskSettings {
                        bus: DiskBuses::Sata,
                    },
                    ztp: ZtpSettings { method: Ztp::Usb },
                    ..vm
                }),
            )
        }
    }
    /// cJunosEvolved. The vCPUs and memory are its EVO VM's, given to the container as
    /// `CPTX_EVOVM_CPU` and `CPTX_EVOVM_MEM_MB`; they are cJunosEvolved's minimums.
    #[must_use]
    pub fn juniper_evolved_container() -> NodeConfig {
        NodeConfig {
            cpu: CpuSettings { count: 4 },
            ..node_base::network_os_container(
                NodeModel::JuniperEvolved,
                6292,
                CONTAINER_JUNIPER_EVOLVED_REPO,
                InterfaceSettings {
                    data_count: 12,
                    management: MgmtInterfaces::Re0Mgmt0,
                    reserved_count: 0,
                },
            )
        }
    }
    #[must_use]
    pub fn nokia_srlinux() -> NodeConfig {
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::network_os_container(
                NodeModel::NokiaSrlinux,
                4096,
                CONTAINER_NOKIA_SRLINUX_REPO,
                InterfaceSettings {
                    data_count: 34,
                    management: MgmtInterfaces::Mgmt0,
                    reserved_count: 0,
                },
            )
        }
    }
    #[must_use]
    pub fn centos_linux() -> NodeConfig {
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::CentosLinux)
        }
    }
    #[must_use]
    pub fn devbox_linux() -> NodeConfig {
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::DevboxLinux)
        }
    }
    #[must_use]
    pub fn fedora_linux() -> NodeConfig {
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::FedoraLinux)
        }
    }
    #[must_use]
    pub fn redhat_linux() -> NodeConfig {
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::RedhatLinux)
        }
    }
    #[must_use]
    pub fn suse_linux() -> NodeConfig {
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::SuseLinux)
        }
    }
    #[must_use]
    pub fn opensuse_linux() -> NodeConfig {
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Sata,
                },
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::OpensuseLinux)
        }
    }
    #[must_use]
    pub fn ubuntu_linux() -> NodeConfig {
        node_base::cloud_init_vm(NodeModel::UbuntuLinux)
    }
    /// Talos Linux: immutable, API-managed Kubernetes OS with no SSH or shell.
    ///
    /// The machine config is delivered as `NoCloud` `user-data` on a `cidata` CD-ROM.
    #[must_use]
    pub fn talos_linux() -> NodeConfig {
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings { count: 2 },
            settings: KindSettings::VirtualMachine(VmSettings {
                ztp: ZtpSettings {
                    method: Ztp::NoCloud,
                },
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::TalosLinux)
        }
    }
    /// NixOS, built from the image recipe in `bwks/nixos-image`. More memory than the other
    /// distros, because rebuilding the system runs Nix on the node.
    #[must_use]
    pub fn nixos_linux() -> NodeConfig {
        NodeConfig {
            memory: 2048,
            cpu: CpuSettings { count: 2 },
            ..node_base::cloud_init_vm(NodeModel::NixosLinux)
        }
    }
    #[must_use]
    pub fn omarchy_linux() -> NodeConfig {
        let vm = node_base::cloud_init_vm_settings();
        NodeConfig {
            memory: 8192,
            cpu: CpuSettings { count: 4 },
            settings: KindSettings::VirtualMachine(VmSettings {
                machine: MachineSettings {
                    bios: BiosTypes::Uefi,
                    ..vm.machine
                },
                interfaces: InterfaceDeviceSettings {
                    mtu: MTU_STD,
                    ..vm.interfaces
                },
                ..vm
            }),
            ..node_base::cloud_init_vm(NodeModel::OmarchyLinux)
        }
    }
    #[must_use]
    pub fn kali_linux() -> NodeConfig {
        node_base::cloud_init_vm(NodeModel::KaliLinux)
    }
    #[must_use]
    pub fn sonic_linux_vm() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Linux,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::image(
                NodeModel::SonicLinux,
                4096,
                InterfaceSettings {
                    data_count: SONIC_PORT_COUNT,
                    management: MgmtInterfaces::Eth0,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    machine: MachineSettings {
                        kind: MachineType::Q35,
                        ..vm.machine
                    },
                    cdrom: CdromSettings {
                        bus: DiskBuses::Virtio,
                        ..vm.cdrom
                    },
                    ztp: ZtpSettings { method: Ztp::Http },
                    ..vm
                }),
            )
        }
    }
    /// The `docker-sonic-vs` container: 2 CPUs and 1 GiB, of which it uses about 150 MiB idle,
    /// and the VM's 32 front-panel ports, `eth1` to `eth32`.
    #[must_use]
    pub fn sonic_linux_container() -> NodeConfig {
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::network_os_container(
                NodeModel::SonicLinux,
                1024,
                CONTAINER_SONIC_LINUX_REPO,
                InterfaceSettings {
                    data_count: SONIC_PORT_COUNT,
                    management: MgmtInterfaces::Eth0,
                    reserved_count: 0,
                },
            )
        }
    }
    #[must_use]
    pub fn flatcar_linux() -> NodeConfig {
        NodeConfig {
            memory: 2048,
            settings: KindSettings::VirtualMachine(VmSettings {
                ztp: ZtpSettings {
                    method: Ztp::Ignition,
                },
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::FlatcarLinux)
        }
    }
    #[must_use]
    pub fn free_bsd() -> NodeConfig {
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                os_variant: OsVariant::Bsd,
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::FreeBsd)
        }
    }
    #[must_use]
    pub fn open_bsd() -> NodeConfig {
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                os_variant: OsVariant::Bsd,
                ..node_base::cloud_init_vm_settings()
            }),
            ..node_base::cloud_init_vm(NodeModel::OpenBsd)
        }
    }
    #[must_use]
    pub fn devbox_windows() -> NodeConfig {
        let vm = node_base::cloud_init_vm_settings();
        NodeConfig {
            memory: 8192,
            cpu: CpuSettings { count: 4 },
            settings: KindSettings::VirtualMachine(VmSettings {
                os_variant: OsVariant::Server,
                machine: MachineSettings {
                    kind: MachineType::Pc,
                    ..vm.machine
                },
                ..vm
            }),
            ..node_base::cloud_init_vm(NodeModel::DevboxWindows)
        }
    }
    #[must_use]
    pub fn windows_server() -> NodeConfig {
        let vm = node_base::cloud_init_vm_settings();
        NodeConfig {
            memory: 8192,
            cpu: CpuSettings { count: 4 },
            settings: KindSettings::VirtualMachine(VmSettings {
                os_variant: OsVariant::Server,
                machine: MachineSettings {
                    kind: MachineType::Pc,
                    ..vm.machine
                },
                ..vm
            }),
            ..node_base::cloud_init_vm(NodeModel::WindowsServer)
        }
    }
    #[must_use]
    pub fn jenkins_server() -> NodeConfig {
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings { count: 2 },
            ..node_base::cloud_init_vm(NodeModel::JenkinsServer)
        }
    }
    #[must_use]
    pub fn nautobot_server() -> NodeConfig {
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings { count: 2 },
            ..node_base::cloud_init_vm(NodeModel::NautobotServer)
        }
    }
    #[must_use]
    pub fn virt_server() -> NodeConfig {
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings { count: 2 },
            ..node_base::cloud_init_vm(NodeModel::VirtServer)
        }
    }
    #[must_use]
    pub fn netbox_server() -> NodeConfig {
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings { count: 2 },
            ..node_base::cloud_init_vm(NodeModel::NetboxServer)
        }
    }
    #[must_use]
    pub fn infrahub_server() -> NodeConfig {
        NodeConfig {
            memory: 12288,
            cpu: CpuSettings { count: 6 },
            ..node_base::cloud_init_vm(NodeModel::InfrahubServer)
        }
    }
    #[must_use]
    pub fn signoz_server() -> NodeConfig {
        NodeConfig {
            memory: 8192,
            cpu: CpuSettings { count: 4 },
            ..node_base::cloud_init_vm(NodeModel::SignozServer)
        }
    }
    #[must_use]
    pub fn forgejo_forge() -> NodeConfig {
        let base = node_base::app_container(NodeModel::ForgejoForge, Some(CONTAINER_FORGEJO_REPO));
        NodeConfig {
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn gitlab_ce() -> NodeConfig {
        let base = node_base::app_container(NodeModel::GitlabCe, Some(CONTAINER_GITLAB_CE_REPO));
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings { count: 2 },
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn surreal_db() -> NodeConfig {
        let base = node_base::app_container(NodeModel::SurrealDb, Some(CONTAINER_SURREAL_DB_REPO));
        NodeConfig {
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn hashicorp_vault() -> NodeConfig {
        let base = node_base::app_container(
            NodeModel::HashicorpVault,
            Some(CONTAINER_HASHICORP_VAULT_REPO),
        );
        NodeConfig {
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn keycloak_server() -> NodeConfig {
        let base =
            node_base::app_container(NodeModel::KeycloakServer, Some(CONTAINER_KEYCLOAK_REPO));
        NodeConfig {
            memory: 2048,
            cpu: CpuSettings { count: 2 },
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn mysql_db() -> NodeConfig {
        node_base::app_container(NodeModel::MysqlDb, None)
    }
    #[must_use]
    pub fn mongo_db() -> NodeConfig {
        node_base::app_container(NodeModel::MongoDb, Some(CONTAINER_MONGO_DB_REPO))
    }
    #[must_use]
    pub fn nats_bus() -> NodeConfig {
        NodeConfig {
            memory: 512,
            ..node_base::app_container(NodeModel::NatsBus, Some(CONTAINER_NATS_REPO))
        }
    }
    #[must_use]
    pub fn postgresql_db() -> NodeConfig {
        node_base::app_container(NodeModel::PostgresqlDb, None)
    }
    #[must_use]
    pub fn mikrotik_chr() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Routeros,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
            },
        );
        node_base::image(
            NodeModel::MikrotikChr,
            1024,
            InterfaceSettings {
                data_count: 8,
                management: MgmtInterfaces::Ether1,
                reserved_count: 0,
            },
            KindSettings::VirtualMachine(VmSettings {
                boot_disk: BootDiskSettings {
                    bus: DiskBuses::Ide,
                },
                cdrom: CdromSettings {
                    bus: DiskBuses::Ide,
                    ..vm.cdrom
                },
                ztp: ZtpSettings { method: Ztp::Disk },
                ..vm
            }),
        )
    }
    #[must_use]
    pub fn paloalto_panos() -> NodeConfig {
        let vm = node_base::network_os_vm_settings(
            OsVariant::Panos,
            InterfaceDeviceSettings {
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
            },
        );
        NodeConfig {
            cpu: CpuSettings { count: 2 },
            ..node_base::image(
                NodeModel::PaloaltoPanos,
                8192,
                InterfaceSettings {
                    data_count: 24,
                    management: MgmtInterfaces::Management,
                    reserved_count: 0,
                },
                KindSettings::VirtualMachine(VmSettings {
                    machine: MachineSettings {
                        kind: MachineType::Q35,
                        ..vm.machine
                    },
                    ..vm
                }),
            )
        }
    }
    #[must_use]
    pub fn frr_linux() -> NodeConfig {
        node_base::network_os_container(
            NodeModel::FrrLinux,
            512,
            CONTAINER_FRR_REPO,
            InterfaceSettings {
                data_count: 8,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
        )
    }
    #[must_use]
    pub fn generic_container() -> NodeConfig {
        node_base::app_container(NodeModel::Generic, None)
    }
    #[must_use]
    pub fn generic_unikernel() -> NodeConfig {
        node_base::unikernel(NodeModel::Generic, UnikernelBootMode::DirectKernel)
    }
    #[must_use]
    pub fn unikraft_unikernel() -> NodeConfig {
        node_base::unikernel(
            NodeModel::UnikraftUnikernel,
            UnikernelBootMode::DirectKernel,
        )
    }
    #[must_use]
    pub fn nanos_unikernel() -> NodeConfig {
        node_base::unikernel(NodeModel::NanosUnikernel, UnikernelBootMode::DiskBoot)
    }
    #[must_use]
    pub fn generic_vm() -> NodeConfig {
        let vm = node_base::cloud_init_vm_settings();
        NodeConfig {
            settings: KindSettings::VirtualMachine(VmSettings {
                os_variant: OsVariant::Unknown,
                machine: MachineSettings {
                    kind: MachineType::Pc,
                    ..vm.machine
                },
                interfaces: InterfaceDeviceSettings {
                    mtu: MTU_STD,
                    ..vm.interfaces
                },
                ..vm
            }),
            ..node_base::cloud_init_vm(NodeModel::Generic)
        }
    }
}

/// The interfaces of both IOL models: `Ethernet0/0` for management and 15 data ports.
fn cisco_iol_interfaces() -> InterfaceSettings {
    InterfaceSettings {
        data_count: 15,
        management: MgmtInterfaces::Ethernet0_0,
        reserved_count: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konst::TALOS_API_PORT;

    fn pulled(model: NodeModel, repo: Option<&str>) -> NodeConfig {
        let mut config = ModelKind::new(model, Some(NodeKind::Container))
            .unwrap()
            .node_config();
        config.settings = KindSettings::Container(ContainerSettings {
            repo: repo.map(str::to_string),
            ztp: ZtpSettings::default(),
        });
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
        let config = pulled(NodeModel::Generic, Some("alpine"));
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
        let unikernel_models = vec![NodeModel::UnikraftUnikernel, NodeModel::NanosUnikernel];
        for model in unikernel_models {
            let config = ModelKind::default_for(model).node_config();
            assert_eq!(
                config.kind(),
                NodeKind::Unikernel,
                "Model {model} should be Unikernel kind"
            );
        }
    }

    #[test]
    fn test_unikernel_config_boot_modes() {
        // DirectKernel models
        for model in [NodeModel::Generic, NodeModel::UnikraftUnikernel] {
            let config = ModelKind::new(model, Some(NodeKind::Unikernel))
                .unwrap()
                .node_config();
            assert_eq!(
                config.unikernel().map(|u| u.boot_mode.clone()),
                Some(UnikernelBootMode::DirectKernel),
                "Model {model} should default to DirectKernel"
            );
        }

        // DiskBoot model
        let nanos_config = ModelKind::default_for(NodeModel::NanosUnikernel).node_config();
        assert_eq!(
            nanos_config.unikernel().map(|u| u.boot_mode.clone()),
            Some(UnikernelBootMode::DiskBoot),
            "NanosUnikernel should default to DiskBoot"
        );
    }

    #[test]
    fn test_unikernel_config_no_ztp() {
        let unikernel_models = vec![
            NodeModel::Generic,
            NodeModel::UnikraftUnikernel,
            NodeModel::NanosUnikernel,
        ];
        for model in unikernel_models {
            let config = ModelKind::new(model, Some(NodeKind::Unikernel))
                .unwrap()
                .node_config();
            assert_eq!(
                config.ztp(),
                Ztp::Disabled,
                "Model {model} should have ZTP disabled"
            );
        }
    }

    #[test]
    fn test_unikernel_config_defaults() {
        let config = ModelKind::default_for(NodeModel::UnikraftUnikernel).node_config();
        assert_eq!(config.memory, 512);
        assert_eq!(config.cpu.count, 1);
        let unikernel = config.unikernel().unwrap();
        assert_eq!(unikernel.interfaces.kind.to_string(), "virtio");
        assert_eq!(unikernel.interfaces.mtu, MTU_STD);
        assert_eq!(unikernel.machine.kind, MachineType::Pc);
    }

    #[test]
    fn test_non_unikernel_models_have_no_boot_mode() {
        let vm_models = vec![
            NodeModel::CiscoNexus9300,
            NodeModel::AristaEos,
            NodeModel::UbuntuLinux,
        ];
        for model in vm_models {
            let config = ModelKind::default_for(model).node_config();
            assert!(
                config.unikernel().is_none(),
                "VM model {model} should not have unikernel settings"
            );
        }
    }
    #[test]
    fn test_omarchy_model_defaults() {
        let model: super::NodeModel = "omarchy_linux".parse().unwrap();
        let config = super::ModelKind::default_for(model).node_config();
        assert_eq!(model.to_string(), "omarchy_linux");
        let vm = config.vm().unwrap();
        assert!(matches!(vm.machine.bios, super::BiosTypes::Uefi));
        assert_eq!(config.ztp(), super::Ztp::CloudInit);
        assert!(matches!(vm.boot_disk.bus, super::DiskBuses::Virtio));
        assert!(vm.cdrom.image.is_none());
        assert_eq!(config.memory, 8192);
        assert_eq!(config.cpu.count, 4);
        let encoded = serde_json::to_string(&model).unwrap();
        assert_eq!(
            serde_json::from_str::<super::NodeModel>(&encoded).unwrap(),
            model
        );
    }

    #[test]
    fn sonic_linux_has_one_data_interface_per_front_panel_port() {
        let config = ModelKind::default_for(NodeModel::SonicLinux).node_config();
        assert_eq!(config.interfaces.data_count, 32);
        assert_eq!(config.interfaces.management, MgmtInterfaces::Eth0);
    }

    #[test]
    fn sonic_linux_comes_as_a_vm_by_default_and_as_a_container() {
        assert_eq!(
            NodeModel::SonicLinux.kinds(),
            [NodeKind::VirtualMachine, NodeKind::Container]
        );
        assert_eq!(
            ModelKind::new(NodeModel::SonicLinux, None).unwrap().kind(),
            NodeKind::VirtualMachine
        );
    }

    #[test]
    fn the_sonic_container_is_a_network_os_container_from_docker_sonic_vs() {
        let config = ModelKind::new(NodeModel::SonicLinux, Some(NodeKind::Container))
            .unwrap()
            .node_config();
        assert_eq!(config.model, NodeModel::SonicLinux);
        assert_eq!(config.kind(), NodeKind::Container);
        assert_eq!(config.cpu.count, 2);
        assert_eq!(config.memory, 1024);
        assert_eq!(config.interfaces.data_count, 32);
        assert_eq!(config.interfaces.reserved_count, 0);
        assert_eq!(config.interfaces.management, MgmtInterfaces::Eth0);
        assert_eq!(config.ztp(), Ztp::Volume);
        assert_eq!(
            config.container_repo(None).as_deref(),
            Some("docker-sonic-vs")
        );
    }

    #[test]
    fn cisco_iol_models_are_network_os_containers_with_16_ethernet_ports() {
        for (name, repo) in [
            ("cisco_iol", "localrepo/cisco_iol"),
            ("cisco_ioll2", "localrepo/cisco_ioll2"),
        ] {
            let model: NodeModel = name.parse().unwrap();
            assert_eq!(model.to_string(), name);
            let config = ModelKind::default_for(model).node_config();
            assert_eq!(config.model, model);
            assert_eq!(config.kind(), NodeKind::Container, "{name}");
            assert_eq!(config.memory, 768, "{name}");
            assert_eq!(config.cpu.count, 1, "{name}");
            assert_eq!(config.interfaces.data_count, 15, "{name}");
            assert_eq!(config.interfaces.reserved_count, 0, "{name}");
            assert_eq!(
                config.interfaces.management,
                MgmtInterfaces::Ethernet0_0,
                "{name}"
            );
            assert_eq!(
                config.container().unwrap().repo.as_deref(),
                Some(repo),
                "{name}"
            );
            assert_eq!(config.ztp(), Ztp::Volume, "{name}");
            assert_eq!(
                ModelKind::default_for(model).ztp_methods(),
                &[Ztp::Volume],
                "{name}"
            );
            let encoded = serde_json::to_string(&model).unwrap();
            assert_eq!(serde_json::from_str::<NodeModel>(&encoded).unwrap(), model);
        }
    }

    #[test]
    fn test_nixos_model_defaults() {
        let model: super::NodeModel = "nixos_linux".parse().unwrap();
        let config = super::ModelKind::default_for(model).node_config();
        assert_eq!(model.to_string(), "nixos_linux");
        assert_eq!(config.kind(), super::NodeKind::VirtualMachine);
        let vm = config.vm().unwrap();
        assert!(matches!(vm.os_variant, super::OsVariant::Linux));
        assert!(matches!(vm.machine.bios, super::BiosTypes::SeaBios));
        assert_eq!(config.ztp(), super::Ztp::CloudInit);
        assert!(matches!(vm.boot_disk.bus, super::DiskBuses::Virtio));
        assert!(matches!(vm.interfaces.kind, super::InterfaceType::Virtio));
        assert!(matches!(
            config.interfaces.management,
            super::MgmtInterfaces::Eth0
        ));
        assert!(vm.cdrom.image.is_none());
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
        let config = super::ModelKind::default_for(model).node_config();
        assert_eq!(model.to_string(), "talos_linux");
        assert_eq!(config.kind(), super::NodeKind::VirtualMachine);
        let vm = config.vm().unwrap();
        assert!(matches!(vm.os_variant, super::OsVariant::Linux));
        assert!(matches!(vm.machine.kind, super::MachineType::Q35));
        assert!(matches!(vm.cpu.model, super::CpuModels::HostModel));
        assert_eq!(config.ztp(), super::Ztp::NoCloud);
        assert!(matches!(vm.boot_disk.bus, super::DiskBuses::Virtio));
        assert!(matches!(vm.cdrom.bus, super::DiskBuses::Sata));
        assert!(matches!(vm.interfaces.kind, super::InterfaceType::Virtio));
        assert!(vm.cdrom.image.is_none());
        assert_eq!(config.cpu.count, 2);
        assert_eq!(config.memory, 4096);
        let model_kind = ModelKind::default_for(model);
        assert_eq!(model_kind.default_boot_disk_size(), Some(20));
        assert!(!model_kind.has_ssh());
        assert_eq!(model_kind.ready_port(), Some(TALOS_API_PORT));
        let encoded = serde_json::to_string(&model).unwrap();
        assert_eq!(
            serde_json::from_str::<super::NodeModel>(&encoded).unwrap(),
            model
        );
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

    #[test]
    fn test_container_and_unikernel_default_ztp() {
        let mounted = [
            NodeModel::AristaEos,
            NodeModel::NokiaSrlinux,
            NodeModel::JuniperEvolved,
            NodeModel::CiscoIol,
            NodeModel::CiscoIoll2,
            NodeModel::FrrLinux,
            NodeModel::SonicLinux,
        ];
        for model_kind in ModelKind::all() {
            let model = model_kind.model();
            let config = model_kind.node_config();
            let expected = match config.kind() {
                NodeKind::Container if mounted.contains(&model) => Ztp::Volume,
                NodeKind::Container | NodeKind::Unikernel => Ztp::Disabled,
                NodeKind::VirtualMachine => continue,
            };
            assert_eq!(config.ztp(), expected, "default of {model}");
        }
    }

    /// Every image's JSON holds the shared groups and exactly its kind's group, with exactly
    /// these fields in each, and reads back to the same JSON.
    #[test]
    fn test_node_config_json_holds_the_shared_groups_and_its_kinds_group() {
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
        let shared = [
            ("cpu", vec!["count"]),
            (
                "interfaces",
                vec!["data_count", "management", "reserved_count"],
            ),
        ];
        for model_kind in ModelKind::all() {
            let model = format!("{} {}", model_kind.model(), model_kind.kind());
            let json = serde_json::to_value(model_kind.node_config()).unwrap();
            let group = kind_settings::group_of(model_kind.kind());
            let mut top = vec![
                "cpu",
                "default",
                "id",
                "interfaces",
                "kind",
                "memory",
                "model",
                "version",
                group,
            ];
            top.sort_unstable();
            assert_eq!(keys(&json), top, "{model}");
            assert_eq!(json["kind"], model_kind.kind().to_string(), "{model}");
            for (name, fields) in &shared {
                assert_eq!(keys(&json[name]), *fields, "{model} {name}");
            }
            let own: Vec<(&str, Vec<&str>)> = match model_kind.kind() {
                NodeKind::VirtualMachine => vec![
                    (
                        "",
                        vec![
                            "boot_disk",
                            "cdrom",
                            "cpu",
                            "interfaces",
                            "machine",
                            "os_variant",
                            "ztp",
                        ],
                    ),
                    ("cpu", vec!["architecture", "model", "vmx"]),
                    ("machine", vec!["bios", "kind"]),
                    ("boot_disk", vec!["bus"]),
                    ("cdrom", vec!["bus", "image"]),
                    ("interfaces", vec!["kind", "mtu"]),
                    ("ztp", vec!["method"]),
                ],
                NodeKind::Container => vec![("", vec!["repo", "ztp"]), ("ztp", vec!["method"])],
                NodeKind::Unikernel => vec![
                    ("", vec!["boot_mode", "cpu", "interfaces", "machine"]),
                    ("cpu", vec!["architecture", "model"]),
                    ("machine", vec!["kind"]),
                    ("interfaces", vec!["kind", "mtu"]),
                ],
            };
            for (name, fields) in own {
                let value = if name.is_empty() {
                    &json[group]
                } else {
                    &json[group][name]
                };
                assert_eq!(keys(value), fields, "{model} {group}.{name}");
            }
            let back: NodeConfig = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(back).unwrap(), json, "{model}");
        }
    }

    #[test]
    fn an_image_json_with_another_kinds_group_is_refused() {
        let mut json = serde_json::to_value(NodeConfig::ubuntu_linux()).unwrap();
        json["container"] = serde_json::to_value(ContainerSettings {
            repo: None,
            ztp: ZtpSettings::default(),
        })
        .unwrap();
        let error = serde_json::from_value::<NodeConfig>(json)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("is a virtual_machine image, which does not take the `container` group"),
            "{error}"
        );
    }

    #[test]
    fn an_image_json_without_its_kinds_group_is_refused() {
        let mut json = serde_json::to_value(NodeConfig::ubuntu_linux()).unwrap();
        json["kind"] = serde_json::json!("container");
        let error = serde_json::from_value::<NodeConfig>(json)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not take the `vm` group"), "{error}");
        let mut json = serde_json::to_value(NodeConfig::mysql_db()).unwrap();
        json.as_object_mut().unwrap().remove("container");
        let error = serde_json::from_value::<NodeConfig>(json)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("is a container image and needs its `container` group"),
            "{error}"
        );
    }

    #[test]
    fn a_container_image_json_has_no_vm_settings() {
        for removed in ["machine", "boot_disk", "cdrom", "os_variant", "repo"] {
            let mut json = serde_json::to_value(NodeConfig::mysql_db()).unwrap();
            json[removed] = serde_json::json!({});
            let error = serde_json::from_value::<NodeConfig>(json)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("unknown field `{removed}`")),
                "{removed}: {error}"
            );
        }
        for removed in ["architecture", "model", "vmx"] {
            let mut json = serde_json::to_value(NodeConfig::mysql_db()).unwrap();
            json["cpu"][removed] = serde_json::json!("x");
            assert!(
                serde_json::from_value::<NodeConfig>(json).is_err(),
                "{removed}"
            );
        }
        for removed in ["kind", "mtu", "prefix", "first_index"] {
            let mut json = serde_json::to_value(NodeConfig::mysql_db()).unwrap();
            json["interfaces"][removed] = serde_json::json!(1);
            assert!(
                serde_json::from_value::<NodeConfig>(json).is_err(),
                "{removed}"
            );
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

    #[test]
    fn test_node_config_json_refuses_the_removed_ztp_login_fields() {
        for (field, value) in [
            ("username", serde_json::json!("admin")),
            ("password", serde_json::json!("Secret-Pass1")),
            ("password_auth", serde_json::json!(true)),
        ] {
            let mut json = serde_json::to_value(NodeConfig::ubuntu_linux()).unwrap();
            json["vm"]["ztp"][field] = value;
            let error = serde_json::from_value::<NodeConfig>(json)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("unknown field `{field}`")),
                "{field}: {error}"
            );
        }
    }

    #[test]
    fn every_pairs_settings_are_its_kinds_group() {
        for model_kind in ModelKind::all() {
            let config = model_kind.node_config();
            assert_eq!(config.kind(), model_kind.kind(), "{model_kind:?}");
            assert_eq!(
                config.vm().is_some(),
                model_kind.kind() == NodeKind::VirtualMachine
            );
            assert_eq!(
                config.container().is_some(),
                model_kind.kind() == NodeKind::Container
            );
            assert_eq!(
                config.unikernel().is_some(),
                model_kind.kind() == NodeKind::Unikernel
            );
            assert_eq!(
                config.interface_device().is_some(),
                model_kind.kind() != NodeKind::Container
            );
        }
    }

    #[test]
    fn every_model_name_round_trips_through_display_fromstr_serde_and_the_command_line() {
        let command = clap::Command::new("test").arg(
            clap::Arg::new("model")
                .long("model")
                .value_parser(NodeModel::value_parser()),
        );
        for model in NodeModel::iter() {
            let name = model.to_string();
            assert_eq!(name.parse::<NodeModel>().unwrap(), model);
            assert_eq!(
                serde_json::to_value(model).unwrap(),
                serde_json::json!(name)
            );
            assert_eq!(
                serde_json::from_value::<NodeModel>(serde_json::json!(name)).unwrap(),
                model
            );
            let matches = command
                .clone()
                .try_get_matches_from(["test", "--model", &name])
                .unwrap();
            assert_eq!(matches.get_one::<NodeModel>("model"), Some(&model));
        }
    }

    #[test]
    fn every_removed_model_name_is_refused_naming_its_model_and_kind() {
        let command = clap::Command::new("test").arg(
            clap::Arg::new("model")
                .long("model")
                .value_parser(NodeModel::value_parser()),
        );
        for (name, model, kind) in REMOVED_MODELS {
            assert!(model.kinds().contains(&kind), "{name}");
            let expected = format!(
                "Unknown node model '{name}': it was renamed in 0.7.0. \
                 Use model = \"{model}\" with kind = \"{kind}\"."
            );
            assert_eq!(name.parse::<NodeModel>().unwrap_err(), expected);
            let error = serde_json::from_value::<NodeModel>(serde_json::json!(name))
                .unwrap_err()
                .to_string();
            assert_eq!(error, expected);
            let error = command
                .clone()
                .try_get_matches_from(["test", "--model", name])
                .unwrap_err()
                .to_string();
            assert!(error.contains(&expected), "{error}");
        }
        assert_eq!(
            "arista_ceos".parse::<NodeModel>().unwrap_err(),
            "Unknown node model 'arista_ceos': it was renamed in 0.7.0. \
             Use model = \"arista_eos\" with kind = \"container\"."
        );
        assert_eq!(
            "cisco_iosv".parse::<NodeModel>().unwrap_err(),
            "Unknown node model 'cisco_iosv': it was renamed in 0.7.0. \
             Use model = \"cisco_ios\" with kind = \"virtual_machine\"."
        );
        assert_eq!(
            "nosuch_model".parse::<NodeModel>().unwrap_err(),
            "Unknown node model: nosuch_model"
        );
    }

    #[test]
    fn names_never_released_are_unknown() {
        for name in ["cisco_iosxr", "juniper_srx"] {
            assert_eq!(
                name.parse::<NodeModel>().unwrap_err(),
                format!("Unknown node model: {name}")
            );
        }
    }

    #[test]
    fn models_renamed_in_0_7_0_have_their_new_names() {
        let renamed = [
            ("cisco_asav", "cisco_asa"),
            ("cisco_ftdv", "cisco_ftd"),
            ("cisco_iosxrv9000", "cisco_iosxr9000"),
            ("cisco_iosv", "cisco_ios"),
            ("cisco_iosvl2", "cisco_iosl2"),
            ("cisco_csr1000v", "cisco_csr1000"),
            ("cisco_cat8000v", "cisco_cat8000"),
            ("cisco_cat9000v", "cisco_cat9000"),
            ("cisco_nexus9300v", "cisco_nexus9300"),
            ("juniper_vsrxv3", "juniper_srxv3"),
            ("juniper_vrouter", "juniper_router"),
            ("juniper_vswitch", "juniper_switch"),
            ("arista_veos", "arista_eos"),
            ("arista_ceos", "arista_eos"),
            ("juniper_vevolved", "juniper_evolved"),
            ("juniper_cevolved", "juniper_evolved"),
            ("generic_vm", "generic"),
            ("generic_container", "generic"),
            ("generic_unikernel", "generic"),
        ];
        assert_eq!(renamed.len(), REMOVED_MODELS.len());
        for (old, new) in renamed {
            let (_, model, _) = REMOVED_MODELS
                .iter()
                .find(|(name, _, _)| *name == old)
                .unwrap();
            assert_eq!(model.to_string(), new, "{old}");
        }
    }

    #[test]
    fn no_removed_model_name_is_a_model() {
        for (name, _, _) in REMOVED_MODELS {
            assert!(
                NodeModel::iter().all(|model| model.to_string() != name),
                "{name}"
            );
        }
    }

    #[test]
    fn the_command_line_lists_every_model_as_a_possible_value() {
        let values: Vec<String> = NodeModelParser
            .possible_values()
            .unwrap()
            .map(|value| value.get_name().to_string())
            .collect();
        let names: Vec<String> = NodeModel::iter().map(|model| model.to_string()).collect();
        assert_eq!(values, names);
    }
}
