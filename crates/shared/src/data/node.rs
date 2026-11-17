use std::ffi::OsStr;
use std::fmt;
use std::str::FromStr;

use anyhow::Result;
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
    CONTAINER_SURREAL_DB_REPO, MTU_JUMBO_INT, MTU_JUMBO_NET, MTU_STD, SONIC_PORT_COUNT,
};

use super::RecordId;
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
            NodeModel::AristaEos | NodeModel::JuniperEvolved => {
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
            | NodeModel::SonicLinux
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

    /// Whether this kind's nodes run on the image's QEMU machine type, `machine.kind`: virtual
    /// machines and unikernels. A container image carries the default, which nothing reads.
    #[must_use]
    pub fn uses_machine_type(&self) -> bool {
        matches!(self, NodeKind::VirtualMachine | NodeKind::Unikernel)
    }

    /// Whether this kind's nodes use the image's firmware, boot disk bus and CD-ROM:
    /// `machine.bios`, `boot_disk` and `cdrom`. Only virtual machines do; container and
    /// unikernel images carry the defaults, which nothing reads.
    #[must_use]
    pub fn uses_firmware_and_disks(&self) -> bool {
        matches!(self, NodeKind::VirtualMachine)
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
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZtpSettings {
    /// `disabled`, or how the image's nodes are provisioned at boot.
    pub method: Ztp,
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MachineSettings {
    /// The QEMU machine type.
    pub kind: MachineType,
    pub bios: BiosTypes,
}

/// The disk a node image boots from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BootDiskSettings {
    /// The bus the boot disk attaches to.
    pub bus: DiskBuses,
}

/// The CD-ROM a node image boots with, if any, and the bus CD-ROMs attach to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
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
    /// The model and kind of this image.
    ///
    /// # Errors
    ///
    /// Fails when the image's model does not take its kind, as a record edited by hand could.
    pub fn model_kind(&self) -> Result<ModelKind> {
        ModelKind::new(self.model, Some(self.kind))
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
    pub fn arista_eos_vm() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::AristaEos,
            2048,
            OsVariant::Eos,
            InterfaceSettings {
                data_count: 52,
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
                prefix: "Eth".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Management1,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                image: Some("aboot.iso".to_owned()),
                bus: DiskBuses::Ide,
            },
            ztp: ZtpSettings { method: Ztp::Tftp },
            ..base
        }
    }
    #[must_use]
    pub fn arista_eos_container() -> NodeConfig {
        let base = node_base::network_os_container(
            NodeModel::AristaEos,
            4096,
            OsVariant::Eos,
            InterfaceSettings {
                data_count: 52,
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
                prefix: "eth".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            repo: Some(CONTAINER_ARISTA_EOS_REPO.to_string()),
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn aruba_aoscx() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::ArubaAoscx,
            4096,
            OsVariant::Aos,
            InterfaceSettings {
                data_count: 52,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "1/1/".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Mgmt,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                ..base.machine
            },
            ztp: ZtpSettings { method: Ztp::Tftp },
            ..base
        }
    }
    #[must_use]
    pub fn cisco_asa() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoAsa,
            2048,
            OsVariant::Asa,
            InterfaceSettings {
                data_count: 8,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "GigabitEthernet0".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                bus: DiskBuses::Ide,
                ..base.cdrom
            },
            ..base
        }
    }
    #[must_use]
    pub fn cisco_csr1000() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoCsr1000,
            3072,
            OsVariant::Iosxe,
            InterfaceSettings {
                data_count: 15,
                kind: InterfaceType::Vmxnet3,
                mtu: MTU_JUMBO_INT,
                prefix: "Gig".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::GigabitEthernet1,
                reserved_count: 0,
            },
        );
        NodeConfig {
            machine: MachineSettings {
                kind: MachineType::Q35,
                ..base.machine
            },
            ..base
        }
    }
    #[must_use]
    pub fn cisco_cat8000() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoCat8000,
            4096,
            OsVariant::Iosxe,
            InterfaceSettings {
                data_count: 15,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "Gig".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::GigabitEthernet1,
                reserved_count: 0,
            },
        );
        NodeConfig {
            machine: MachineSettings {
                kind: MachineType::Q35,
                ..base.machine
            },
            ..base
        }
    }
    #[must_use]
    pub fn cisco_cat9000() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoCat9000,
            18432,
            OsVariant::Iosxe,
            InterfaceSettings {
                data_count: 8,
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
                prefix: "Gig0/0/".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::GigabitEthernet0_0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 4,
                ..base.cpu
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                ..base.machine
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ..base
        }
    }
    #[must_use]
    pub fn cisco_iosxr9000() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoIosxr9000,
            20480,
            OsVariant::Iosxr,
            InterfaceSettings {
                data_count: 31,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "Gig0/0/0/".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::MgmtEth0Rp0Cpu0_0,
                reserved_count: 2,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 4,
                ..base.cpu
            },
            machine: MachineSettings {
                bios: BiosTypes::Uefi,
                ..base.machine
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ..base
        }
    }
    #[must_use]
    pub fn cisco_nexus9300() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoNexus9300,
            12288,
            OsVariant::Nxos,
            InterfaceSettings {
                data_count: 64,
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
                prefix: "Eth1/".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Mgmt0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            machine: MachineSettings {
                bios: BiosTypes::Uefi,
                ..base.machine
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            cdrom: CdromSettings {
                bus: DiskBuses::Ide,
                ..base.cdrom
            },
            ..base
        }
    }
    #[must_use]
    pub fn cisco_ios() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoIos,
            768,
            OsVariant::Ios,
            InterfaceSettings {
                data_count: 15,
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
                prefix: "Gig0/".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::GigabitEthernet0_0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cdrom: CdromSettings {
                bus: DiskBuses::Virtio,
                ..base.cdrom
            },
            ztp: ZtpSettings { method: Ztp::Disk },
            ..base
        }
    }
    #[must_use]
    pub fn cisco_iosl2() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoIosl2,
            1024,
            OsVariant::Ios,
            InterfaceSettings {
                // IOSvL2 crashes with more than 16 NICs: GigabitEthernet0/0 for management plus 15 data.
                data_count: 15,
                kind: InterfaceType::E1000,
                mtu: MTU_JUMBO_INT,
                prefix: "Gig".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::GigabitEthernet0_0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cdrom: CdromSettings {
                bus: DiskBuses::Virtio,
                ..base.cdrom
            },
            ztp: ZtpSettings { method: Ztp::Disk },
            ..base
        }
    }
    /// Cisco IOL XE, from the user's CML image rebuilt by docker-cisco-iol. 16 Ethernet ports in
    /// slots of 4: `Ethernet0/0` for management and 15 data ports.
    #[must_use]
    pub fn cisco_iol() -> NodeConfig {
        NodeConfig {
            repo: Some(CONTAINER_CISCO_IOL_REPO.to_string()),
            ..node_base::network_os_container(
                NodeModel::CiscoIol,
                768,
                OsVariant::Ios,
                cisco_iol_interfaces(),
            )
        }
    }
    /// Cisco IOL XE L2, the switch image; as `cisco_iol`.
    #[must_use]
    pub fn cisco_ioll2() -> NodeConfig {
        NodeConfig {
            repo: Some(CONTAINER_CISCO_IOLL2_REPO.to_string()),
            ..node_base::network_os_container(
                NodeModel::CiscoIoll2,
                768,
                OsVariant::Ios,
                cisco_iol_interfaces(),
            )
        }
    }
    #[must_use]
    pub fn cisco_ise() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoIse,
            16384,
            OsVariant::Ise,
            InterfaceSettings {
                data_count: 1,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "eth".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 4,
                ..base.cpu
            },
            ztp: ZtpSettings { method: Ztp::Disk },
            ..base
        }
    }
    #[must_use]
    pub fn cisco_ftd() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::CiscoFtd,
            8192,
            OsVariant::Fxos,
            InterfaceSettings {
                data_count: 8,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "gig0/".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::Management0_0,
                reserved_count: 1,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 4,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn juniper_router() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::JuniperRouter,
            5120,
            OsVariant::Junos,
            InterfaceSettings {
                data_count: 10,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
                prefix: "ge-0/0/".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::Fxp0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 4,
                model: CpuModels::IvyBridge,
                vmx: true,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn juniper_switch() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::JuniperSwitch,
            5120,
            OsVariant::Junos,
            InterfaceSettings {
                data_count: 10,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
                prefix: "ge-0/0/".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::Fxp0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 4,
                model: CpuModels::IvyBridge,
                vmx: true,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn juniper_evolved_vm() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::JuniperEvolved,
            8192,
            OsVariant::Junos,
            InterfaceSettings {
                data_count: 12,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
                prefix: "et-0/0/".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::Re0Mgmt0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 4,
                model: CpuModels::IvyBridge,
                vmx: true,
                ..base.cpu
            },
            machine: MachineSettings {
                bios: BiosTypes::Uefi,
                ..base.machine
            },
            ztp: ZtpSettings { method: Ztp::Tftp },
            ..base
        }
    }
    #[must_use]
    pub fn juniper_srxv3() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::JuniperSrxv3,
            4096,
            OsVariant::Junos,
            InterfaceSettings {
                data_count: 8,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_NET,
                prefix: "ge-0/0/".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::Fxp0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 2,
                model: CpuModels::SandyBridge,
                vmx: true,
                ..base.cpu
            },
            machine: MachineSettings {
                kind: MachineType::PcI440Fx8_0,
                ..base.machine
            },
            cdrom: CdromSettings {
                bus: DiskBuses::Ide,
                ..base.cdrom
            },
            ..base
        }
    }
    #[must_use]
    pub fn alma_linux() -> NodeConfig {
        NodeConfig {
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ..node_base::cloud_init_vm(NodeModel::AlmaLinux)
        }
    }
    #[must_use]
    pub fn rocky_linux() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::RockyLinux);
        NodeConfig {
            machine: MachineSettings {
                bios: BiosTypes::Uefi,
                ..base.machine
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ..base
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
        let base = node_base::network_os_vm(
            NodeModel::CumulusLinux,
            2048,
            OsVariant::Nvue,
            InterfaceSettings {
                data_count: 52,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "swp".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                ..base.machine
            },
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ztp: ZtpSettings { method: Ztp::Usb },
            ..base
        }
    }
    /// cJunosEvolved. The vCPUs and memory are its EVO VM's, given to the container as
    /// `CPTX_EVOVM_CPU` and `CPTX_EVOVM_MEM_MB`; they are cJunosEvolved's minimums.
    #[must_use]
    pub fn juniper_evolved_container() -> NodeConfig {
        let base = node_base::network_os_container(
            NodeModel::JuniperEvolved,
            6292,
            OsVariant::Junos,
            InterfaceSettings {
                data_count: 12,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "et-0/0/".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Re0Mgmt0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            repo: Some(CONTAINER_JUNIPER_EVOLVED_REPO.to_string()),
            cpu: CpuSettings {
                count: 4,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn nokia_srlinux() -> NodeConfig {
        let base = node_base::network_os_container(
            NodeModel::NokiaSrlinux,
            4096,
            OsVariant::Srlinux,
            InterfaceSettings {
                data_count: 34,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "eth-1/".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Mgmt0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            repo: Some(CONTAINER_NOKIA_SRLINUX_REPO.to_string()),
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn centos_linux() -> NodeConfig {
        NodeConfig {
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ..node_base::cloud_init_vm(NodeModel::CentosLinux)
        }
    }
    #[must_use]
    pub fn devbox_linux() -> NodeConfig {
        NodeConfig {
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ..node_base::cloud_init_vm(NodeModel::DevboxLinux)
        }
    }
    #[must_use]
    pub fn fedora_linux() -> NodeConfig {
        NodeConfig {
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ..node_base::cloud_init_vm(NodeModel::FedoraLinux)
        }
    }
    #[must_use]
    pub fn redhat_linux() -> NodeConfig {
        NodeConfig {
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ..node_base::cloud_init_vm(NodeModel::RedhatLinux)
        }
    }
    #[must_use]
    pub fn suse_linux() -> NodeConfig {
        NodeConfig {
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
            ..node_base::cloud_init_vm(NodeModel::SuseLinux)
        }
    }
    #[must_use]
    pub fn opensuse_linux() -> NodeConfig {
        NodeConfig {
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Sata,
            },
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
        let base = node_base::cloud_init_vm(NodeModel::TalosLinux);
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            ztp: ZtpSettings {
                method: Ztp::NoCloud,
            },
            ..base
        }
    }
    /// NixOS, built from the image recipe in `bwks/nixos-image`. More memory than the other
    /// distros, because rebuilding the system runs Nix on the node.
    #[must_use]
    pub fn nixos_linux() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::NixosLinux);
        NodeConfig {
            memory: 2048,
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn omarchy_linux() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::OmarchyLinux);
        NodeConfig {
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                ..base.cpu
            },
            machine: MachineSettings {
                bios: BiosTypes::Uefi,
                ..base.machine
            },
            interfaces: InterfaceSettings {
                mtu: MTU_STD,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn kali_linux() -> NodeConfig {
        node_base::cloud_init_vm(NodeModel::KaliLinux)
    }
    #[must_use]
    pub fn sonic_linux() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::SonicLinux,
            4096,
            OsVariant::Linux,
            InterfaceSettings {
                data_count: SONIC_PORT_COUNT,
                kind: InterfaceType::Virtio,
                mtu: MTU_JUMBO_INT,
                prefix: "eth".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                ..base.machine
            },
            cdrom: CdromSettings {
                bus: DiskBuses::Virtio,
                ..base.cdrom
            },
            ztp: ZtpSettings { method: Ztp::Http },
            ..base
        }
    }
    #[must_use]
    pub fn flatcar_linux() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::FlatcarLinux);
        NodeConfig {
            memory: 2048,
            ztp: ZtpSettings {
                method: Ztp::Ignition,
            },
            ..base
        }
    }
    #[must_use]
    pub fn free_bsd() -> NodeConfig {
        NodeConfig {
            os_variant: OsVariant::Bsd,
            ..node_base::cloud_init_vm(NodeModel::FreeBsd)
        }
    }
    #[must_use]
    pub fn open_bsd() -> NodeConfig {
        NodeConfig {
            os_variant: OsVariant::Bsd,
            ..node_base::cloud_init_vm(NodeModel::OpenBsd)
        }
    }
    #[must_use]
    pub fn devbox_windows() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::DevboxWindows);
        NodeConfig {
            os_variant: OsVariant::Server,
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                ..base.cpu
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                ..base.machine
            },
            ..base
        }
    }
    #[must_use]
    pub fn windows_server() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::WindowsServer);
        NodeConfig {
            os_variant: OsVariant::Server,
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                ..base.cpu
            },
            machine: MachineSettings {
                kind: MachineType::Pc,
                ..base.machine
            },
            ..base
        }
    }
    #[must_use]
    pub fn jenkins_server() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::JenkinsServer);
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn nautobot_server() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::NautobotServer);
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn virt_server() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::VirtServer);
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn netbox_server() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::NetboxServer);
        NodeConfig {
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn infrahub_server() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::InfrahubServer);
        NodeConfig {
            memory: 12288,
            cpu: CpuSettings {
                count: 6,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn signoz_server() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::SignozServer);
        NodeConfig {
            memory: 8192,
            cpu: CpuSettings {
                count: 4,
                ..base.cpu
            },
            ..base
        }
    }
    #[must_use]
    pub fn forgejo_forge() -> NodeConfig {
        let base = node_base::app_container(NodeModel::ForgejoForge);
        NodeConfig {
            repo: Some(CONTAINER_FORGEJO_REPO.to_string()),
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn gitlab_ce() -> NodeConfig {
        let base = node_base::app_container(NodeModel::GitlabCe);
        NodeConfig {
            repo: Some(CONTAINER_GITLAB_CE_REPO.to_string()),
            memory: 4096,
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn surreal_db() -> NodeConfig {
        let base = node_base::app_container(NodeModel::SurrealDb);
        NodeConfig {
            repo: Some(CONTAINER_SURREAL_DB_REPO.to_string()),
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn hashicorp_vault() -> NodeConfig {
        let base = node_base::app_container(NodeModel::HashicorpVault);
        NodeConfig {
            repo: Some(CONTAINER_HASHICORP_VAULT_REPO.to_string()),
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn keycloak_server() -> NodeConfig {
        let base = node_base::app_container(NodeModel::KeycloakServer);
        NodeConfig {
            repo: Some(CONTAINER_KEYCLOAK_REPO.to_string()),
            memory: 2048,
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            interfaces: InterfaceSettings {
                data_count: 0,
                ..base.interfaces
            },
            ..base
        }
    }
    #[must_use]
    pub fn mysql_db() -> NodeConfig {
        node_base::app_container(NodeModel::MysqlDb)
    }
    #[must_use]
    pub fn mongo_db() -> NodeConfig {
        NodeConfig {
            repo: Some(CONTAINER_MONGO_DB_REPO.to_string()),
            ..node_base::app_container(NodeModel::MongoDb)
        }
    }
    #[must_use]
    pub fn nats_bus() -> NodeConfig {
        NodeConfig {
            repo: Some(CONTAINER_NATS_REPO.to_string()),
            memory: 512,
            ..node_base::app_container(NodeModel::NatsBus)
        }
    }
    #[must_use]
    pub fn postgresql_db() -> NodeConfig {
        node_base::app_container(NodeModel::PostgresqlDb)
    }
    #[must_use]
    pub fn mikrotik_chr() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::MikrotikChr,
            1024,
            OsVariant::Routeros,
            InterfaceSettings {
                data_count: 8,
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
                prefix: "ether".to_owned(),
                first_index: 1,
                management: MgmtInterfaces::Ether1,
                reserved_count: 0,
            },
        );
        NodeConfig {
            boot_disk: BootDiskSettings {
                bus: DiskBuses::Ide,
            },
            cdrom: CdromSettings {
                bus: DiskBuses::Ide,
                ..base.cdrom
            },
            ztp: ZtpSettings { method: Ztp::Disk },
            ..base
        }
    }
    #[must_use]
    pub fn paloalto_panos() -> NodeConfig {
        let base = node_base::network_os_vm(
            NodeModel::PaloaltoPanos,
            8192,
            OsVariant::Panos,
            InterfaceSettings {
                data_count: 24,
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
                prefix: "ethernet1/".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::Management,
                reserved_count: 0,
            },
        );
        NodeConfig {
            cpu: CpuSettings {
                count: 2,
                ..base.cpu
            },
            machine: MachineSettings {
                kind: MachineType::Q35,
                ..base.machine
            },
            ..base
        }
    }
    #[must_use]
    pub fn frr_linux() -> NodeConfig {
        let base = node_base::network_os_container(
            NodeModel::FrrLinux,
            512,
            OsVariant::Linux,
            InterfaceSettings {
                data_count: 8,
                kind: InterfaceType::Virtio,
                mtu: MTU_STD,
                prefix: "eth".to_owned(),
                first_index: 0,
                management: MgmtInterfaces::Eth0,
                reserved_count: 0,
            },
        );
        NodeConfig {
            repo: Some(CONTAINER_FRR_REPO.to_string()),
            ..base
        }
    }
    #[must_use]
    pub fn generic_container() -> NodeConfig {
        node_base::app_container(NodeModel::Generic)
    }
    #[must_use]
    pub fn generic_unikernel() -> NodeConfig {
        node_base::unikernel(NodeModel::Generic)
    }
    #[must_use]
    pub fn unikraft_unikernel() -> NodeConfig {
        node_base::unikernel(NodeModel::UnikraftUnikernel)
    }
    #[must_use]
    pub fn nanos_unikernel() -> NodeConfig {
        NodeConfig {
            unikernel: Some(UnikernelSettings {
                boot_mode: UnikernelBootMode::DiskBoot,
            }),
            ..node_base::unikernel(NodeModel::NanosUnikernel)
        }
    }
    #[must_use]
    pub fn generic_vm() -> NodeConfig {
        let base = node_base::cloud_init_vm(NodeModel::Generic);
        NodeConfig {
            os_variant: OsVariant::Unknown,
            machine: MachineSettings {
                kind: MachineType::Pc,
                ..base.machine
            },
            interfaces: InterfaceSettings {
                mtu: MTU_STD,
                ..base.interfaces
            },
            ..base
        }
    }
}

/// The interfaces of both IOL models. IOL's MTU is fixed at 1500.
fn cisco_iol_interfaces() -> InterfaceSettings {
    InterfaceSettings {
        data_count: 15,
        kind: InterfaceType::Virtio,
        mtu: MTU_STD,
        prefix: "Ethernet".to_owned(),
        first_index: 1,
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
                config.kind,
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
                config.unikernel.map(|u| u.boot_mode),
                Some(UnikernelBootMode::DirectKernel),
                "Model {model} should default to DirectKernel"
            );
        }

        // DiskBoot model
        let nanos_config = ModelKind::default_for(NodeModel::NanosUnikernel).node_config();
        assert_eq!(
            nanos_config.unikernel.map(|u| u.boot_mode),
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
                config.ztp.method,
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
        assert_eq!(config.interfaces.kind.to_string(), "virtio");
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
                config.unikernel.is_none(),
                "VM model {model} should not have unikernel settings"
            );
        }
    }
    #[test]
    fn test_omarchy_model_defaults() {
        let model: super::NodeModel = "omarchy_linux".parse().unwrap();
        let config = super::ModelKind::default_for(model).node_config();
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
    fn sonic_linux_has_one_data_interface_per_front_panel_port() {
        let config = ModelKind::default_for(NodeModel::SonicLinux).node_config();
        assert_eq!(config.interfaces.data_count, 32);
        assert_eq!(config.interfaces.prefix, "eth");
        assert_eq!(config.interfaces.first_index, 1);
        assert_eq!(config.interfaces.management, MgmtInterfaces::Eth0);
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
            assert_eq!(config.kind, NodeKind::Container, "{name}");
            assert!(matches!(config.os_variant, OsVariant::Ios), "{name}");
            assert_eq!(config.memory, 768, "{name}");
            assert_eq!(config.cpu.count, 1, "{name}");
            assert_eq!(config.interfaces.data_count, 15, "{name}");
            assert_eq!(config.interfaces.mtu, MTU_STD, "{name}");
            assert_eq!(config.interfaces.prefix, "Ethernet", "{name}");
            assert_eq!(config.interfaces.first_index, 1, "{name}");
            assert_eq!(config.interfaces.reserved_count, 0, "{name}");
            assert_eq!(
                config.interfaces.management,
                MgmtInterfaces::Ethernet0_0,
                "{name}"
            );
            assert_eq!(config.repo.as_deref(), Some(repo), "{name}");
            assert_eq!(config.ztp.method, Ztp::Volume, "{name}");
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
        let config = super::ModelKind::default_for(model).node_config();
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
        ];
        for model_kind in ModelKind::all() {
            let model = model_kind.model();
            let config = model_kind.node_config();
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
            ("ztp", vec!["method"]),
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
        for model_kind in ModelKind::all() {
            let model = format!("{} {}", model_kind.model(), model_kind.kind());
            let json = serde_json::to_value(model_kind.node_config()).unwrap();
            assert_eq!(keys(&json), top, "{model}");
            for (group, fields) in &groups {
                assert_eq!(keys(&json[group]), *fields, "{model} {group}");
            }
            match model_kind.kind() {
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

    #[test]
    fn test_node_config_json_refuses_the_removed_ztp_login_fields() {
        for (field, value) in [
            ("username", serde_json::json!("admin")),
            ("password", serde_json::json!("Secret-Pass1")),
            ("password_auth", serde_json::json!(true)),
        ] {
            let mut json = serde_json::to_value(NodeConfig::ubuntu_linux()).unwrap();
            json["ztp"][field] = value;
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
    fn only_virtual_machines_and_unikernels_use_the_machine_type() {
        assert!(NodeKind::VirtualMachine.uses_machine_type());
        assert!(NodeKind::Unikernel.uses_machine_type());
        assert!(!NodeKind::Container.uses_machine_type());
    }

    #[test]
    fn only_virtual_machines_use_the_firmware_and_disks() {
        assert!(NodeKind::VirtualMachine.uses_firmware_and_disks());
        assert!(!NodeKind::Unikernel.uses_firmware_and_disks());
        assert!(!NodeKind::Container.uses_firmware_and_disks());
    }

    #[test]
    fn models_carry_the_defaults_for_the_settings_their_kind_does_not_use() {
        for model in NodeModel::to_vec() {
            let config = ModelKind::default_for(model).node_config();
            if !config.kind.uses_machine_type() {
                assert_eq!(config.machine.kind, MachineType::default(), "{model}");
            }
            if !config.kind.uses_firmware_and_disks() {
                assert_eq!(config.machine.bios, BiosTypes::default(), "{model}");
                assert_eq!(config.boot_disk, BootDiskSettings::default(), "{model}");
                assert_eq!(config.cdrom, CdromSettings::default(), "{model}");
            }
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
