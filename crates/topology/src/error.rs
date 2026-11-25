use std::fmt;

use thiserror::Error;

use shared::SharedError;
use shared::data::{DiskBuses, InterfaceType, ModelKind, NodeKind, NodeModel, PtxPlatform, Ztp};
use shared::konst::{
    CONTAINER_SECRETS_DIR, JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB, NODE_USER_ENV_DIR,
};

/// A manifest check's failure that a caller tells apart from the others.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum TopologyError {
    #[error("Failed to parse manifest {path}")]
    ManifestUnparsable { path: String },

    #[error("Manifest - `description` is {length} characters long; the limit is {limit}")]
    DescriptionTooLong { length: usize, limit: usize },

    #[error(
        "Manifest - `description` holds the control character {character:?}; only line breaks and tabs are allowed"
    )]
    DescriptionControlCharacter { character: char },

    #[error("Manifest - node: '{node}' defined more than once")]
    DuplicateNode { node: String },

    #[error(
        "Manifest - nodes '{node}' and '{other}' differ only in case; DNS takes them as one name"
    )]
    NodeNamesDifferInCase { node: String, other: String },

    #[error("Manifest - {count} nodes defined, but a lab supports at most {max} nodes")]
    TooManyNodes { count: usize, max: usize },

    #[error(
        "Manifest - {what} name '{name}' is not allowed: use 1 to {max_len} letters, \
         digits, '-' or '_', starting with a letter or digit"
    )]
    NameNotPlain {
        what: &'static str,
        name: String,
        max_len: usize,
    },

    #[error(
        "Manifest - node name '{name}' is not allowed: '{word}' is a \
         reserved word in node names, because the SONiC image treats a host whose name \
         contains it as a chassis supervisor card"
    )]
    ReservedNodeName { name: String, word: &'static str },

    #[error("Invalid environment variable '{entry}' for node '{node}': must be KEY=VALUE format")]
    EnvironmentVariableNotKeyValue { entry: String, node: String },

    #[error("Invalid environment variable '{entry}' for node '{node}': KEY must not be empty")]
    EnvironmentVariableEmptyKey { entry: String, node: String },

    #[error(
        "Invalid environment variable key '{key}' for node '{node}': KEY must not start with a digit"
    )]
    EnvironmentVariableKeyStartsWithDigit { key: String, node: String },

    #[error(
        "Invalid environment variable key '{key}' for node '{node}': KEY must contain only alphanumeric characters and underscores"
    )]
    EnvironmentVariableKeyNotAName { key: String, node: String },

    #[error(
        "Manifest - node: '{node}' lists {listed} {field} but {read} were read; the Sherpa \
         CLI reads these files on the client, so start the lab with `sherpa up`"
    )]
    FilesNotRead {
        node: String,
        field: &'static str,
        listed: usize,
        read: usize,
    },

    #[error("The manifest as written names node '{node}', which is not in the manifest")]
    AsWrittenUnknownNode { node: String },

    #[error("The file contents name node '{node}', which is not in the manifest")]
    FileContentsUnknownNode { node: String },

    #[error("Manifest - node: '{node}' `{field}` is 0; it must be at least 1")]
    ZeroSetting { node: String, field: &'static str },

    #[error("Manifest - `ready_timeout` is 0; it must be at least 1 second")]
    ReadyTimeoutZero,

    #[error("Manifest - `ready_timeout` is {seconds}; it must be at most {max} seconds", max = i64::MAX)]
    ReadyTimeoutTooLong { seconds: u64 },

    #[error("Manifest - node: '{node}'")]
    NodeModelKind { node: String },

    #[error(
        "Manifest - node: '{node}' sets no `kind`, which '{model}' needs; set it to one of: {}",
        joined(kinds)
    )]
    KindMissing {
        node: String,
        model: NodeModel,
        kinds: Vec<NodeKind>,
    },

    #[error(
        r#"Manifest - node: '{node}' sets `kind = "{kind}"`, but '{model}' comes only as: {}"#,
        joined(kinds)
    )]
    KindNotTaken {
        node: String,
        kind: NodeKind,
        model: NodeModel,
        kinds: Vec<NodeKind>,
    },

    #[error(
        "Manifest - node: '{node}' sets `{key}`, which does not apply to {kind} nodes; '{model}' is a {kind}{}",
        unless(*other)
    )]
    SettingNotForKind {
        node: String,
        key: String,
        kind: NodeKind,
        model: NodeModel,
        other: Option<NodeKind>,
    },

    #[error(
        "Manifest - node: '{node}' sets {key}, which {} with its own `ztp.config`",
        uses(*model, *kind)
    )]
    FileSettingWithOwnConfig {
        node: String,
        key: &'static str,
        model: NodeModel,
        kind: NodeKind,
    },

    #[error(
        "Manifest - node: '{node}' sets {key}, which {} with ZTP disabled",
        uses(*model, *kind)
    )]
    FileSettingWithZtpDisabled {
        node: String,
        key: &'static str,
        model: NodeModel,
        kind: NodeKind,
    },

    #[error("Manifest - node: '{node}' sets {key}, which {}", uses(*model, *kind))]
    FileSettingNotUsed {
        node: String,
        key: &'static str,
        model: NodeModel,
        kind: NodeKind,
    },

    #[error(
        "Manifest - node: '{node}' sets `container.platform`, which is only for a juniper_evolved container, not {model} ({kind})"
    )]
    PlatformNotForModel {
        node: String,
        model: NodeModel,
        kind: NodeKind,
    },

    #[error(
        "Manifest - node: '{node}' has {interfaces} data interfaces, but platform {platform} has at most {max}"
    )]
    TooManyPlatformInterfaces {
        node: String,
        interfaces: u8,
        platform: PtxPlatform,
        max: u8,
    },

    #[error(
        "Manifest - node: '{node}' has {cpus} vCPUs; a juniper_evolved container needs at least {min}"
    )]
    TooFewPtxCpus { node: String, cpus: u8, min: u8 },

    #[error(
        "Manifest - node: '{node}' has {memory} MB of memory; a juniper_evolved container needs at least {min}"
    )]
    TooLittlePtxMemory { node: String, memory: u16, min: u16 },

    #[error(
        "Manifest - node: '{node}' has {memory} MB of memory; a juniper_evolved container takes at most {max}, as its \
         container needs {overhead} MB more",
        overhead = JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB
    )]
    TooMuchPtxMemory { node: String, memory: u16, max: u16 },

    #[error(
        "tailscale.auth_key_env must name an environment variable (letters, digits, underscores; not starting with a digit)"
    )]
    InvalidAuthKeyEnv,

    #[error(
        "Manifest - node: '{node}' is a container node with the Tailscale gateway's name; rename it, or disable [tailscale]"
    )]
    TailnetGatewayName { node: String },

    #[error("Manifest - node: '{node}' text file '{dst}' permissions are invalid")]
    TextFilePermissions {
        node: String,
        dst: String,
        #[source]
        mode: SharedError,
    },

    #[error(
        r#"Manifest - node: '{node}' startup script '{script}' contains '"' or '%', which cannot be used in a script name on a Windows node"#
    )]
    WindowsScriptName { node: String, script: String },

    #[error(
        "Manifest - node: '{node}' sets `ztp.config`, but its ZTP is disabled, so the config \
         would not be used"
    )]
    ZtpConfigWithZtpDisabled { node: String },

    #[error(
        "Manifest - node: '{node}' sets `ztp.config`, which ZTP method '{ztp}' does not use. \
         Only tftp, cdrom, disk, usb, nocloud and volume take a custom ZTP config"
    )]
    ZtpConfigNotUsed { node: String, ztp: Ztp },

    #[error(
        "Manifest - node: '{node}' sets `cpu.topology`, `cpu.pinning` or \
         `interfaces.queues`; set `cpu.count` too"
    )]
    CpuCountNeeded { node: String },

    #[error(
        "Manifest - node: '{node}' `cpu.topology` gives {vcpus} vCPUs, but `cpu.count` \
         is {cpu_count}"
    )]
    TopologyMismatch {
        node: String,
        vcpus: u32,
        cpu_count: u32,
    },

    #[error(
        "Manifest - node: '{node}' `cpu.pinning` lists {listed} host CPUs, one is needed for \
         each of its {cpu_count} vCPUs"
    )]
    PinningCount {
        node: String,
        listed: usize,
        cpu_count: u32,
    },

    #[error("Manifest - node: '{node}' `cpu.pinning` lists host CPU {cpu} more than once")]
    PinningRepeat { node: String, cpu: u16 },

    #[error("Manifest - node: '{node}' `{setting}` is empty; list host CPUs or remove it")]
    EmptyCpuList { node: String, setting: &'static str },

    #[error(
        "Manifest - node: '{node}' `{setting}` uses host CPU {cpu}, which is pinned to \
         one of its vCPUs"
    )]
    CpuPinnedToVcpu {
        node: String,
        setting: &'static str,
        cpu: u16,
    },

    #[error(
        "Manifest - node: '{node}' sets `numa.memory_mode` without `numa.node`; set the \
         host NUMA node its memory is placed on, or remove `numa.memory_mode`"
    )]
    NumaModeWithoutNode { node: String },

    #[error(
        r#"Manifest - node: '{node}' sets `boot_disk.io = "native"`, which needs `boot_disk.cache = "none"`"#
    )]
    NativeIoWithoutNoCache { node: String },

    #[error(
        "Manifest - node: '{node}' sets `boot_disk.io_thread_cpus`, which needs virtio \
         disks; '{model}' uses {bus}"
    )]
    IoThreadsNeedVirtio {
        node: String,
        model: NodeModel,
        bus: DiskBuses,
    },

    #[error(
        "Manifest - node: '{node}' `interfaces.queues` must be between 1 and \
         {cpu_count}, its `cpu.count`"
    )]
    QueuesOutOfRange { node: String, cpu_count: u32 },

    #[error(
        "Manifest - node: '{node}' sets `interfaces.queues`, which needs virtio \
         interfaces; '{model}' uses {interfaces:?}"
    )]
    QueuesNeedVirtio {
        node: String,
        model: NodeModel,
        interfaces: InterfaceType,
    },

    #[error("Host CPU {cpu} is pinned to vCPUs of both node '{node}' and node '{other}'")]
    VcpuPinShared {
        cpu: u16,
        node: String,
        other: String,
    },

    #[error(
        "Host CPU {cpu} runs QEMU threads of node '{node}' but is pinned to vCPUs of \
         node '{other}'"
    )]
    QemuThreadsOnVcpu {
        cpu: u16,
        node: String,
        other: String,
    },

    #[error("Manifest - talos cluster '{cluster}' is defined more than once")]
    DuplicateCluster { cluster: String },

    #[error(
        "Manifest - talos cluster '{cluster}' has {controllers} controllers; a cluster needs 1 or 3, because etcd needs a majority of controllers to stay up"
    )]
    ControllerCount { cluster: String, controllers: usize },

    #[error(
        "Manifest - talos cluster '{cluster}' lists node '{node}', which is not defined in nodes"
    )]
    UnknownClusterMember { cluster: String, node: String },

    #[error(
        "Manifest - talos cluster '{cluster}' lists node '{node}', a {model} node; only talos_linux nodes can join a cluster"
    )]
    ClusterMemberNotTalos {
        cluster: String,
        node: String,
        model: NodeModel,
    },

    #[error(
        "Manifest - node '{node}' sets ztp.config but joins talos cluster '{cluster}'; Sherpa generates the machine configs of cluster nodes"
    )]
    ClusterMemberOwnZtpConfig { node: String, cluster: String },

    #[error(
        "Manifest - node '{node}' is listed more than once in talos_clusters; a node joins one cluster, as a controller or a worker"
    )]
    ClusterMemberRepeated { node: String },

    #[error(
        "Manifest - talos cluster '{cluster}' sets {field} = '{node}', which is not defined in nodes"
    )]
    UnknownClusterService {
        cluster: String,
        field: &'static str,
        node: String,
    },

    #[error(
        "Manifest - talos cluster '{cluster}' sets {field} = '{node}', a {model} node; it must be a {expected} node"
    )]
    ClusterServiceModel {
        cluster: String,
        field: &'static str,
        node: String,
        model: NodeModel,
        expected: NodeModel,
    },

    #[error(
        "Manifest - talos cluster '{cluster}' sets vault = '{vault}', but that node sets ztp.config; Sherpa configures only the Vault it initialises with its own config"
    )]
    ClusterVaultOwnZtpConfig { cluster: String, vault: String },

    #[error(
        "Manifest - {clusters} talos clusters have 3 controllers, but a lab has only {addresses} shared cluster addresses"
    )]
    TooManyHaClusters { clusters: usize, addresses: usize },

    #[error(
        "Manifest - {clusters} talos clusters use cilium, but a lab has load balancer addresses for only {blocks}"
    )]
    TooManyCiliumClusters { clusters: usize, blocks: usize },

    #[error("provision_concurrency must be at least 1")]
    ProvisionConcurrencyZero,

    #[error("Node '{node}' has an ssh provisioner, which runs only on Linux and BSD VMs")]
    SshProvisionerNotSupported { node: String },

    #[error("Node '{node}' has a container provisioner, which runs only on containers")]
    ContainerProvisionerNotSupported { node: String },

    #[error("Node '{node}': a container provisioner has nothing to do; add files")]
    ContainerProvisionerEmpty { node: String },

    #[error("Node '{node}': a container secret file has an empty src")]
    SecretFileEmptySrc { node: String },

    #[error(
        "Node '{node}': two secret files are named '{name}', and both would be mounted at \
         {CONTAINER_SECRETS_DIR}/{name}"
    )]
    DuplicateSecretFile { node: String, name: String },

    #[error("Node '{node}': an ssh provisioner has nothing to do; add files, env_files or scripts")]
    SshProvisionerEmpty { node: String },

    #[error("Node '{node}': a provisioner file has an empty src")]
    ProvisionerFileEmptySrc { node: String },

    #[error(
        "Node '{node}': provisioner file dst '{dst}' must be an absolute path or start with ~/, \
         name a file, use only letters, digits and . _ - + @, and not contain empty, . or .. \
         parts"
    )]
    InvalidProvisionerDestination { node: String, dst: String },

    #[error("Node '{node}': provisioner file '{dst}' mode is invalid")]
    ProvisionerFileMode { node: String, dst: String },

    #[error("Node '{node}': env file '{env_file}' does not name a file")]
    EnvFileNotAFile { node: String, env_file: String },

    #[error(
        "Node '{node}': two env files are named '{name}', and both would be copied to \
         ~/{NODE_USER_ENV_DIR}/{name}"
    )]
    DuplicateEnvFile { node: String, name: String },

    #[error("Node '{node}': a provisioner script path is empty")]
    EmptyScriptPath { node: String },

    #[error("Manifest link - '{node}' defined in links, not defined in nodes")]
    UnknownLinkNode { node: String },

    #[error("Manifest bridge - node '{node}' defined in bridges, not defined in nodes")]
    UnknownBridgeNode { node: String },

    #[error("Manifest link - '{src}' to '{dst}': each end must be written node::interface")]
    LinkEndNotNodeInterface { src: String, dst: String },

    #[error("Manifest bridge - '{bridge}': link '{link}' must be written node::interface")]
    BridgeLinkNotNodeInterface { bridge: String, link: String },

    #[error("Manifest link - '{src}' to '{dst}': both ends are on node '{node}'")]
    LinkToItself {
        src: String,
        dst: String,
        node: String,
    },

    #[error("Manifest bridge - '{bridge}' has {links} links; a bridge needs at least 2")]
    BridgeTooFewLinks { bridge: String, links: usize },

    #[error("Manifest bridge - '{bridge}' defined more than once")]
    DuplicateBridge { bridge: String },

    #[error(
        "Manifest bridge - name '{bridge}' is not allowed: a bridge name follows the node name rules"
    )]
    BridgeNameNotAllowed { bridge: String },

    #[error("{at}: node '{node}' ({model_kind}) has no interface '{interface}'")]
    NoSuchInterface {
        at: LinkPlace,
        node: String,
        model_kind: ModelKind,
        interface: String,
    },

    #[error("Manifest link - {src} <-> {dst}")]
    LinkImpairment { src: String, dst: String },

    #[error(
        "Manifest {} - node '{node}' interface index '{index}' is already in use",
        connection.named()
    )]
    InterfaceInUse {
        connection: Connection,
        node: String,
        index: u8,
    },

    #[error(
        "{} is the management interface and cannot be used in {}",
        connection.place(node, interface),
        connection.plural()
    )]
    ManagementInterface {
        connection: Connection,
        node: String,
        interface: String,
    },

    #[error(
        "{} is a reserved interface (reserved_count: {reserved}) and cannot be used in {}",
        connection.place(node, interface),
        connection.plural()
    )]
    ReservedInterface {
        connection: Connection,
        node: String,
        interface: String,
        reserved: u8,
    },

    #[error(
        "{} exceeds the maximum interface index '{max_index}' for node model '{model}' (data_interface_count: {data_interface_count}, reserved: {reserved})",
        connection.place(node, interface)
    )]
    InterfacePastLast {
        connection: Connection,
        node: String,
        interface: String,
        max_index: u8,
        model: NodeModel,
        data_interface_count: u8,
        reserved: u8,
    },
}

/// The manifest link or bridge an interface name was given in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkPlace {
    Link { src: String, dst: String },
    Bridge { name: String },
}

/// What uses a node's interface: a link, or the bridge of that name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    Link,
    Bridge { name: String },
}

impl fmt::Display for LinkPlace {
    /// The link's two ends or the bridge's name, as a message about it starts.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkPlace::Link { src, dst } => write!(f, "Manifest link - '{src}' to '{dst}'"),
            LinkPlace::Bridge { name } => write!(f, "Manifest bridge - '{name}'"),
        }
    }
}

impl Connection {
    /// The connection as one, such as `link` or `bridge 'br1'`.
    fn named(&self) -> String {
        match self {
            Connection::Link => "link".to_owned(),
            Connection::Bridge { name } => format!("bridge '{name}'"),
        }
    }

    /// The kind of connection as many, such as `links`.
    fn plural(&self) -> &'static str {
        match self {
            Connection::Link => "links",
            Connection::Bridge { .. } => "bridges",
        }
    }

    /// The interface `interface` of `node` in this connection, as a message about it starts.
    fn place(&self, node: &str, interface: &str) -> String {
        match self {
            Connection::Link => format!("Manifest link - node '{node}' interface '{interface}'"),
            Connection::Bridge { name } => {
                format!("Manifest bridge '{name}' - node '{node}' interface '{interface}'")
            }
        }
    }
}

/// The kinds, separated by commas.
fn joined(kinds: &[NodeKind]) -> String {
    kinds
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The kind another kind of the node's model would take the setting as, if there is one.
fn unless(other: Option<NodeKind>) -> String {
    other.map_or_else(String::new, |other| {
        format!(r#" unless the node sets `kind = "{other}"`"#)
    })
}

/// That `model` as `kind` does not use a setting, naming the kind only for a model of several.
fn uses(model: NodeModel, kind: NodeKind) -> String {
    if model.kinds().len() > 1 {
        format!("{model} does not use as a {kind}")
    } else {
        format!("{model} does not use")
    }
}
