//! A node model with one of its kinds.
//!
//! What depends on the kind as well as the model hangs off [`ModelKind`]: the default image
//! settings, the ZTP methods, readiness and the boot disk size. A [`ModelKind`] is only built
//! from a model and a kind the model takes, so one in hand is always a valid pair. Each
//! function matches on the model; a model with more than one kind matches on the kind inside its
//! arm.

use std::fmt;

use anyhow::{Result, bail};
use serde_derive::{Deserialize, Serialize};
use strum::IntoEnumIterator;

use super::node::{NodeConfig, NodeKind, NodeModel, Ztp};
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
    type Error = anyhow::Error;

    fn try_from(fields: ModelKindFields) -> Result<Self> {
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
    pub fn new(model: NodeModel, kind: Option<NodeKind>) -> Result<Self> {
        let Some(kind) = kind.or(model.default_kind()) else {
            bail!(
                "Model '{model}' has no default kind; set its kind to one of: {}",
                model_kinds_list(model)
            );
        };
        if !model.kinds().contains(&kind) {
            bail!(
                "Model '{model}' is not a {kind}; it comes only as: {}",
                model_kinds_list(model)
            );
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
    use super::*;

    #[test]
    fn every_model_takes_at_least_one_kind_and_its_default_is_the_first() {
        for model in NodeModel::iter() {
            let kinds = model.kinds();
            assert!(!kinds.is_empty(), "{model} takes no kind");
            assert_eq!(ModelKind::default_for(model).kind(), kinds[0], "{model}");
            let mut unique = kinds.to_vec();
            unique.dedup();
            assert_eq!(unique.len(), kinds.len(), "{model} lists a kind twice");
        }
    }

    #[test]
    fn all_lists_each_model_with_each_of_its_kinds() {
        let pairs: Vec<ModelKind> = ModelKind::all().collect();
        let expected: usize = NodeModel::iter().map(|model| model.kinds().len()).sum();
        assert_eq!(pairs.len(), expected);
        for model_kind in pairs {
            assert!(
                model_kind.model().kinds().contains(&model_kind.kind()),
                "{model_kind:?}"
            );
        }
    }

    #[test]
    fn no_kind_gives_the_models_default_kind() {
        for model in NodeModel::iter().filter(|model| *model != NodeModel::Generic) {
            assert_eq!(
                ModelKind::new(model, None).unwrap(),
                ModelKind::default_for(model),
                "{model}"
            );
        }
    }

    #[test]
    fn generic_has_no_default_kind_and_is_refused_without_one() {
        assert_eq!(NodeModel::Generic.default_kind(), None);
        let error = ModelKind::new(NodeModel::Generic, None)
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "Model 'generic' has no default kind; set its kind to one of: \
             virtual_machine, container, unikernel"
        );
        for &kind in NodeModel::Generic.kinds() {
            assert_eq!(
                ModelKind::new(NodeModel::Generic, Some(kind))
                    .unwrap()
                    .kind(),
                kind
            );
        }
    }

    #[test]
    fn the_merged_models_default_to_their_virtual_machine() {
        for model in [NodeModel::AristaEos, NodeModel::JuniperEvolved] {
            assert_eq!(
                model.kinds(),
                [NodeKind::VirtualMachine, NodeKind::Container],
                "{model}"
            );
            assert_eq!(
                ModelKind::new(model, None).unwrap().kind(),
                NodeKind::VirtualMachine
            );
        }
    }

    #[test]
    fn srx_takes_its_config_from_cdrom_only_and_by_default() {
        let srx = ModelKind::default_for(NodeModel::JuniperSrxv3);
        assert_eq!(srx.ztp_methods(), [Ztp::Cdrom]);
        assert_eq!(srx.node_config().ztp(), Ztp::Cdrom);
    }

    #[test]
    fn each_kind_of_a_merged_model_has_its_own_settings_and_ztp() {
        let veos = ModelKind::default_for(NodeModel::AristaEos);
        let ceos = ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container)).unwrap();
        assert_eq!(veos.ztp_methods(), [Ztp::Tftp]);
        assert_eq!(ceos.ztp_methods(), [Ztp::Volume]);
        assert_eq!(
            ceos.node_config().container_repo(None).as_deref(),
            Some("localrepo/arista_eos")
        );
        assert_eq!(veos.node_config().container_repo(None), None);

        let vevolved = ModelKind::default_for(NodeModel::JuniperEvolved);
        let cevolved =
            ModelKind::new(NodeModel::JuniperEvolved, Some(NodeKind::Container)).unwrap();
        assert_eq!(vevolved.ztp_methods(), [Ztp::Tftp, Ztp::Usb]);
        assert_eq!(cevolved.ztp_methods(), [Ztp::Volume]);
        assert_eq!(
            cevolved.node_config().container_repo(None).as_deref(),
            Some("localrepo/juniper_evolved")
        );
        assert!(cevolved.keeps_data_interfaces_up());
        assert!(!vevolved.keeps_data_interfaces_up());
    }

    #[test]
    fn each_kind_of_sonic_has_its_own_settings_and_ztp() {
        let vm = ModelKind::default_for(NodeModel::SonicLinux);
        let container = ModelKind::new(NodeModel::SonicLinux, Some(NodeKind::Container)).unwrap();
        assert_eq!(vm.kind(), NodeKind::VirtualMachine);
        assert_eq!(vm.ztp_methods(), [Ztp::Http]);
        assert_eq!(container.ztp_methods(), [Ztp::Volume]);
        assert_eq!(vm.node_config().ztp(), Ztp::Http);
        assert_eq!(container.node_config().ztp(), Ztp::Volume);
        assert_eq!(vm.node_config().container_repo(None), None);
        assert_eq!(
            container.node_config().container_repo(None).as_deref(),
            Some("docker-sonic-vs")
        );
        assert!(container.has_ssh());
        assert!(!container.keeps_data_interfaces_up());
        assert_eq!(container.to_string(), "sonic_linux, container");
    }

    #[test]
    fn a_kind_the_model_takes_is_accepted() {
        for model in NodeModel::iter() {
            for &kind in model.kinds() {
                let model_kind = ModelKind::new(model, Some(kind)).unwrap();
                assert_eq!(model_kind.model(), model);
                assert_eq!(model_kind.kind(), kind);
            }
        }
    }

    #[test]
    fn a_kind_the_model_does_not_take_is_refused_naming_the_kinds_it_takes() {
        for model in NodeModel::iter() {
            for kind in NodeKind::iter().filter(|kind| !model.kinds().contains(kind)) {
                let error = ModelKind::new(model, Some(kind)).unwrap_err().to_string();
                assert!(error.contains(&format!("'{model}'")), "{error}");
                assert!(error.contains(&kind.to_string()), "{error}");
                assert!(
                    error.ends_with(&format!("comes only as: {}", model_kinds_list(model))),
                    "{error}"
                );
            }
        }
        let error = ModelKind::new(NodeModel::CiscoAsa, Some(NodeKind::Container))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "Model 'cisco_asa' is not a container; it comes only as: virtual_machine"
        );
    }

    #[test]
    fn a_model_kind_reads_back_from_json_and_a_wrong_pair_is_refused() {
        for model_kind in ModelKind::all() {
            let json = serde_json::to_value(model_kind).unwrap();
            assert_eq!(json["model"], model_kind.model().to_string());
            assert_eq!(json["kind"], model_kind.kind().to_string());
            assert_eq!(
                serde_json::from_value::<ModelKind>(json).unwrap(),
                model_kind
            );
        }
        let json = serde_json::json!({"model": "cisco_asa", "kind": "container"});
        let error = serde_json::from_value::<ModelKind>(json)
            .unwrap_err()
            .to_string();
        assert!(error.contains("comes only as: virtual_machine"), "{error}");
    }

    #[test]
    fn every_pairs_image_settings_are_of_its_model_and_kind() {
        for model_kind in ModelKind::all() {
            let config = model_kind.node_config();
            assert_eq!(config.model, model_kind.model(), "{model_kind:?}");
            assert_eq!(config.kind(), model_kind.kind(), "{model_kind:?}");
            assert_eq!(config.model_kind().unwrap(), model_kind);
        }
    }

    #[test]
    fn an_image_whose_model_does_not_take_its_kind_has_no_model_kind() {
        let mut config = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        config.settings = ModelKind::default_for(NodeModel::MysqlDb)
            .node_config()
            .settings;
        assert!(config.model_kind().is_err());
    }

    #[test]
    fn only_virtual_machines_and_the_sonic_container_have_a_ready_port_of_their_own() {
        let sonic_container =
            ModelKind::new(NodeModel::SonicLinux, Some(NodeKind::Container)).unwrap();
        for model_kind in ModelKind::all() {
            assert_eq!(
                model_kind.ready_port().is_some(),
                model_kind.kind() == NodeKind::VirtualMachine || model_kind == sonic_container,
                "{model_kind:?}"
            );
        }
        assert_eq!(sonic_container.ready_port(), Some(SSH_PORT));
        assert_eq!(
            ModelKind::default_for(NodeModel::TalosLinux).ready_port(),
            Some(TALOS_API_PORT)
        );
    }

    #[test]
    fn only_windows_with_cloud_init_waits_for_sherpa_s_sshd_config() {
        for model_kind in ModelKind::all() {
            let windows = matches!(
                model_kind.model(),
                NodeModel::DevboxWindows | NodeModel::WindowsServer
            );
            for ztp in [Ztp::CloudInit, Ztp::Disabled, Ztp::Tftp] {
                assert_eq!(
                    model_kind.waits_for_sshd_ready_marker(ztp),
                    windows && ztp == Ztp::CloudInit,
                    "{model_kind:?} {ztp:?}"
                );
            }
        }
    }

    #[test]
    fn only_cisco_iol_containers_need_the_host_to_finish_checksums() {
        for model_kind in ModelKind::all() {
            assert_eq!(
                model_kind.needs_host_checksums(),
                matches!(
                    model_kind.model(),
                    NodeModel::CiscoIol | NodeModel::CiscoIoll2
                ),
                "{model_kind:?}"
            );
        }
    }

    #[test]
    fn other_vm_models_keep_ssh_readiness_and_image_disk_size() {
        let ubuntu = ModelKind::default_for(NodeModel::UbuntuLinux);
        assert_eq!(ubuntu.ready_port(), Some(SSH_PORT));
        assert_eq!(
            ModelKind::default_for(NodeModel::FlatcarLinux).ready_port(),
            Some(SSH_PORT)
        );
        assert_eq!(ubuntu.default_boot_disk_size(), None);
        assert!(ubuntu.has_ssh());
    }

    #[test]
    fn test_no_model_lists_disabled_as_a_ztp_method() {
        for model_kind in ModelKind::all() {
            assert!(
                !model_kind.ztp_methods().contains(&Ztp::Disabled),
                "{model_kind:?} lists disabled"
            );
        }
    }

    #[test]
    fn test_every_models_default_ztp_is_disabled_or_a_listed_method() {
        for model_kind in ModelKind::all() {
            let ztp = model_kind.node_config().ztp();
            assert!(
                ztp == Ztp::Disabled || model_kind.ztp_methods().contains(&ztp),
                "{model_kind:?} defaults to {ztp}, which it does not list"
            );
        }
    }

    #[test]
    fn a_model_kind_shows_its_kind_only_for_a_model_with_several() {
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
