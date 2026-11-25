//! A node model with one of its kinds.
//!
//! What depends on the kind as well as the model hangs off [`ModelKind`]: the default image
//! settings, the ZTP methods, readiness and the boot disk size. A [`ModelKind`] is only built
//! from a model and a kind the model takes, so one in hand is always a valid pair. Each
//! function matches on the model; a model with more than one kind matches on the kind inside its
//! arm.

use std::fmt;

use serde_derive::{Deserialize, Serialize};
use strum::IntoEnumIterator;

use super::node::{NodeConfig, NodeKind, NodeModel, Ztp};
use crate::error::SharedError;
use crate::konst::{SSH_PORT, TALOS_API_PORT, TALOS_DEFAULT_BOOT_DISK_SIZE};

/// A node model and one of the kinds it takes. Its fields are private: build one with
/// [`ModelKind::new`], which refuses a kind the model does not take, or with
/// [`ModelKind::default_for`], which takes the model's first kind. Read from JSON, it goes
/// through [`ModelKind::new`] too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "ModelKindFields")]
pub struct ModelKind {
    model: NodeModel,
    kind: NodeKind,
}

/// A [`ModelKind`] as read, before its kind is checked against its model.
#[derive(Deserialize)]
struct ModelKindFields {
    model: NodeModel,
    kind: NodeKind,
}

impl TryFrom<ModelKindFields> for ModelKind {
    type Error = SharedError;

    fn try_from(fields: ModelKindFields) -> Result<Self, SharedError> {
        Self::new(fields.model, Some(fields.kind))
    }
}

/// The model, and its kind when the model takes more than one: `cisco_asa`, but
/// `arista_eos, container`. Messages name a node's model with it.
impl fmt::Display for ModelKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.model.kinds().len() > 1 {
            write!(f, "{}, {}", self.model, self.kind)
        } else {
            write!(f, "{}", self.model)
        }
    }
}

impl Default for ModelKind {
    fn default() -> Self {
        Self::default_for(NodeModel::default())
    }
}

impl ModelKind {
    /// The model with `kind`, or with its default kind when `kind` is `None`.
    ///
    /// # Errors
    ///
    /// Fails when the model does not take `kind`, naming the kinds it takes, and when `kind`
    /// is `None` for a model with no default kind (`generic`).
    pub fn new(model: NodeModel, kind: Option<NodeKind>) -> Result<Self, SharedError> {
        let Some(kind) = kind.or(model.default_kind()) else {
            return Err(SharedError::NoDefaultKind { model });
        };
        if !model.kinds().contains(&kind) {
            return Err(SharedError::KindNotTaken { model, kind });
        }
        Ok(Self { model, kind })
    }

    /// Every model with each kind it takes, in model order and each model's kinds in order.
    pub fn all() -> impl Iterator<Item = ModelKind> {
        NodeModel::iter().flat_map(|model| {
            model
                .kinds()
                .iter()
                .map(move |&kind| ModelKind { model, kind })
        })
    }

    /// The model with the first of [`NodeModel::kinds`], its default kind. `generic` has no
    /// default kind, so this gives its first, `virtual_machine`: code that reads a kind a user
    /// may have left out uses [`ModelKind::new`], which refuses `generic` without one.
    #[must_use]
    pub fn default_for(model: NodeModel) -> Self {
        Self {
            model,
            kind: model.kinds()[0],
        }
    }

    #[must_use]
    pub fn model(self) -> NodeModel {
        self.model
    }

    #[must_use]
    pub fn kind(self) -> NodeKind {
        self.kind
    }

    /// The built-in image settings of this model and kind: what an image starts from when it
    /// is imported, pulled or scanned.
    #[must_use]
    pub fn node_config(self) -> NodeConfig {
        match self.model {
            // Arista
            NodeModel::AristaEos => match self.kind {
                NodeKind::Container => NodeConfig::arista_eos_container(),
                NodeKind::VirtualMachine | NodeKind::Unikernel => NodeConfig::arista_eos_vm(),
            },

            // Aruba
            NodeModel::ArubaAoscx => NodeConfig::aruba_aoscx(),

            // Cisco
            NodeModel::CiscoAsa => NodeConfig::cisco_asa(),
            NodeModel::CiscoCsr1000 => NodeConfig::cisco_csr1000(),
            NodeModel::CiscoCat8000 => NodeConfig::cisco_cat8000(),
            NodeModel::CiscoCat9000 => NodeConfig::cisco_cat9000(),
            NodeModel::CiscoIosxr9000 => NodeConfig::cisco_iosxr9000(),
            NodeModel::CiscoNexus9300 => NodeConfig::cisco_nexus9300(),
            NodeModel::CiscoIos => NodeConfig::cisco_ios(),
            NodeModel::CiscoIosl2 => NodeConfig::cisco_iosl2(),
            NodeModel::CiscoIol => NodeConfig::cisco_iol(),
            NodeModel::CiscoIoll2 => NodeConfig::cisco_ioll2(),
            NodeModel::CiscoIse => NodeConfig::cisco_ise(),
            NodeModel::CiscoFtd => NodeConfig::cisco_ftd(),

            // Juniper
            NodeModel::JuniperRouter => NodeConfig::juniper_router(),
            NodeModel::JuniperSwitch => NodeConfig::juniper_switch(),
            NodeModel::JuniperEvolved => match self.kind {
                NodeKind::Container => NodeConfig::juniper_evolved_container(),
                NodeKind::VirtualMachine | NodeKind::Unikernel => NodeConfig::juniper_evolved_vm(),
            },
            NodeModel::JuniperSrxv3 => NodeConfig::juniper_srxv3(),

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
            NodeModel::SonicLinux => match self.kind {
                NodeKind::Container => NodeConfig::sonic_linux_container(),
                NodeKind::VirtualMachine | NodeKind::Unikernel => NodeConfig::sonic_linux_vm(),
            },
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
            NodeModel::Generic => match self.kind {
                NodeKind::VirtualMachine => NodeConfig::generic_vm(),
                NodeKind::Container => NodeConfig::generic_container(),
                NodeKind::Unikernel => NodeConfig::generic_unikernel(),
            },

            // Unikernels
            NodeModel::UnikraftUnikernel => NodeConfig::unikraft_unikernel(),
            NodeModel::NanosUnikernel => NodeConfig::nanos_unikernel(),
        }
    }

    /// The ZTP methods this model and kind take: one for most, two for a Junos Evolved virtual
    /// machine, and none for those with no config generator. `Ztp::Disabled` is allowed for every pair and is never listed.
    /// Every model is named, so a new one does not compile until it has an entry.
    #[must_use]
    pub fn ztp_methods(self) -> &'static [Ztp] {
        match self.model {
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
            | NodeModel::FreeBsd
            | NodeModel::OpenBsd
            | NodeModel::DevboxWindows
            | NodeModel::WindowsServer
            | NodeModel::AlpineLinux => &[Ztp::CloudInit],
            NodeModel::ArubaAoscx => &[Ztp::Tftp],
            NodeModel::AristaEos => match self.kind {
                NodeKind::Container => &[Ztp::Volume],
                NodeKind::VirtualMachine | NodeKind::Unikernel => &[Ztp::Tftp],
            },
            NodeModel::JuniperEvolved => match self.kind {
                NodeKind::Container => &[Ztp::Volume],
                NodeKind::VirtualMachine | NodeKind::Unikernel => &[Ztp::Tftp, Ztp::Usb],
            },
            NodeModel::Generic => match self.kind {
                NodeKind::VirtualMachine => &[Ztp::CloudInit],
                NodeKind::Container | NodeKind::Unikernel => &[],
            },
            NodeModel::CiscoCsr1000
            | NodeModel::CiscoCat8000
            | NodeModel::CiscoCat9000
            | NodeModel::CiscoAsa
            | NodeModel::CiscoNexus9300
            | NodeModel::CiscoIosxr9000
            | NodeModel::CiscoFtd
            | NodeModel::JuniperSrxv3
            | NodeModel::JuniperRouter
            | NodeModel::JuniperSwitch
            | NodeModel::PaloaltoPanos => &[Ztp::Cdrom],
            NodeModel::CiscoIos
            | NodeModel::CiscoIosl2
            | NodeModel::CiscoIse
            | NodeModel::MikrotikChr => &[Ztp::Disk],
            NodeModel::CumulusLinux => &[Ztp::Usb],
            NodeModel::SonicLinux => match self.kind {
                NodeKind::Container => &[Ztp::Volume],
                NodeKind::VirtualMachine | NodeKind::Unikernel => &[Ztp::Http],
            },
            NodeModel::FlatcarLinux => &[Ztp::Ignition],
            NodeModel::TalosLinux => &[Ztp::NoCloud],
            NodeModel::NokiaSrlinux
            | NodeModel::CiscoIol
            | NodeModel::CiscoIoll2
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
            | NodeModel::UnikraftUnikernel
            | NodeModel::NanosUnikernel => &[],
        }
    }

    /// Whether nodes of this model and kind accept SSH logins. Talos Linux has no SSH or
    /// shell.
    #[must_use]
    pub fn has_ssh(self) -> bool {
        !matches!(self.model, NodeModel::TalosLinux)
    }

    /// Whether every data interface of a container of this model stays up, disabled ones too.
    /// cJunosEvolved, the container of `juniper_evolved`, waits at boot until all the data
    /// interfaces it is told about are up.
    #[must_use]
    pub fn keeps_data_interfaces_up(self) -> bool {
        self.model == NodeModel::JuniperEvolved && self.kind == NodeKind::Container
    }

    /// Whether the host must finish the checksums of packets it sends to a container of this
    /// model, so transmit checksum offload goes off on the host end of its veths. IOL reads its
    /// interfaces through packet sockets, which hand it a packet whose checksum a veth left for
    /// the "hardware" to fill in, and IOL drops it.
    #[must_use]
    pub fn needs_host_checksums(self) -> bool {
        matches!(self.model, NodeModel::CiscoIol | NodeModel::CiscoIoll2)
    }

    /// The TCP port that shows a node of this model and kind is ready, when the manifest sets
    /// none: SSH for a virtual machine, or the Talos API for Talos Linux, which has no SSH.
    /// The `sonic_linux` container has SSH too: Sherpa's start script starts its SSH server last, once
    /// the switch is up and its config loaded. `None` for any other container or a unikernel,
    /// which is ready once it runs.
    #[must_use]
    pub fn ready_port(self) -> Option<u16> {
        match self.kind {
            NodeKind::VirtualMachine => Some(match self.model {
                NodeModel::TalosLinux => TALOS_API_PORT,
                _ => SSH_PORT,
            }),
            NodeKind::Container if self.model == NodeModel::SonicLinux => Some(SSH_PORT),
            NodeKind::Container | NodeKind::Unikernel => None,
        }
    }

    /// Whether a node of this model and kind, configured with `ztp`, must show
    /// [`SHERPA_SSHD_READY_MARKER`](crate::konst::SHERPA_SSHD_READY_MARKER) in its SSH server's
    /// identification line before it counts as ready, on top of the SSH handshake: the owner can
    /// log in only from then. `false` when the handshake is enough. `up` and `redeploy` store it with the node's readiness check, which
    /// every readiness probe reads.
    ///
    /// A Windows node's OpenSSH server starts at boot, before cloudbase-init has created the
    /// owner's account, written their keys file and Sherpa's `sshd_config`, and restarted it.
    /// That config adds the marker; Sherpa writes it only with cloud-init ZTP.
    #[must_use]
    pub fn waits_for_sshd_ready_marker(self, ztp: Ztp) -> bool {
        matches!(
            (self.model, ztp),
            (
                NodeModel::DevboxWindows | NodeModel::WindowsServer,
                Ztp::CloudInit
            )
        )
    }

    /// Boot disk size in GB used when the manifest does not set `boot_disk.size`.
    ///
    /// `None` keeps the image's own size. The Talos Linux image is about 4 GB but Talos needs
    /// at least 10 GB.
    #[must_use]
    pub fn default_boot_disk_size(self) -> Option<u16> {
        match self.model {
            NodeModel::TalosLinux => Some(TALOS_DEFAULT_BOOT_DISK_SIZE),
            _ => None,
        }
    }
}

/// The kinds a model takes, as a comma-separated list for a message.
#[must_use]
pub fn model_kinds_list(model: NodeModel) -> String {
    model
        .kinds()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use serde_json::error::Category;

    use super::*;

    #[test]
    fn node_kind_004_a_model_and_kind_from_json_are_refused_when_the_model_does_not_take_it() {
        let fields = ModelKindFields {
            model: NodeModel::CiscoAsa,
            kind: NodeKind::Container,
        };
        assert!(matches!(
            ModelKind::try_from(fields),
            Err(SharedError::KindNotTaken {
                model: NodeModel::CiscoAsa,
                kind: NodeKind::Container
            })
        ));
        let json = serde_json::json!({"model": "cisco_asa", "kind": "container"});
        let error = serde_json::from_value::<ModelKind>(json).unwrap_err();
        assert_eq!(error.classify(), Category::Data);
    }

    #[test]
    fn node_kind_005_every_model_and_kind_has_built_in_settings_of_that_model_and_kind() {
        for model_kind in ModelKind::all() {
            let config = model_kind.node_config();
            assert_eq!(config.model, model_kind.model(), "{model_kind:?}");
            assert_eq!(config.kind(), model_kind.kind(), "{model_kind:?}");
            assert_eq!(config.model_kind().unwrap(), model_kind);
        }
    }

    #[test]
    fn ztp_setting_013_every_models_built_in_setting_is_disabled_or_a_method_it_takes() {
        for model_kind in ModelKind::all() {
            let ztp = model_kind.node_config().ztp();
            assert!(
                ztp == Ztp::Disabled || model_kind.ztp_methods().contains(&ztp),
                "{model_kind:?} defaults to {ztp}, which it does not list"
            );
        }
    }

    #[test]
    fn node_kind_006_a_message_names_a_model_with_its_kind_only_when_the_model_takes_several() {
        assert_eq!(
            ModelKind::default_for(NodeModel::CiscoAsa).to_string(),
            "cisco_asa"
        );
        assert_eq!(
            ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container))
                .unwrap()
                .to_string(),
            "arista_eos, container"
        );
        assert_eq!(
            ModelKind::default_for(NodeModel::JuniperEvolved).to_string(),
            "juniper_evolved, virtual_machine"
        );
    }
}
