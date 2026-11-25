use std::net::Ipv4Addr;
use std::ops::RangeInclusive;

use ipnet::Ipv4Net;

pub const SHERPA_CONFIG_FILE: &str = "sherpa.toml";
pub const SHERPA_MANIFEST_FILE: &str = "manifest.toml";
pub const SHERPA_CONFIG_DIR: &str = "config";
pub const SHERPA_SSH_DIR: &str = "ssh";
pub const SHERPA_IMAGES_DIR: &str = "images";
pub const SHERPA_CONTAINERS_DIR: &str = "containers";
pub const SHERPA_BINS_DIR: &str = "bins";
pub const VM_DISK_FILENAME: &str = "virtioa.qcow2";
pub const UNIKERNEL_KERNEL_FILENAME: &str = "kernel.elf";
pub const UNIKERNEL_DISK_FILENAME: &str = "disk.qcow2";
// ============================================================================
// Full directory paths
// ============================================================================
pub const SHERPA_IMAGES_PATH: &str = "/opt/sherpa/images";
pub const SHERPA_CONTAINERS_PATH: &str = "/opt/sherpa/containers";
pub const SHERPA_BINS_PATH: &str = "/opt/sherpa/bins";
pub const SHERPA_LABS_PATH: &str = "/opt/sherpa/labs";

pub const SHERPA_STORAGE_POOL: &str = "sherpa-pool";
pub const SHERPA_STORAGE_POOL_PATH: &str = "/opt/sherpa/libvirt/images";
/// Mode of storage pool volumes. libvirt hands them to the QEMU user while a VM runs.
pub const STORAGE_VOLUME_MODE: &str = "0600";
/// Default management network. Built with `new_assert`, so an invalid prefix fails the build.
pub const SHERPA_MANAGEMENT_NETWORK_IPV4: Ipv4Net =
    Ipv4Net::new_assert(Ipv4Addr::new(172, 31, 0, 0), 16);
/// Lab management gateway (libvirt NAT network) host offset.
pub const MANAGEMENT_GATEWAY_HOST_OFFSET: u32 = 1;
/// Lab router (dnsmasq, boot server) host offset.
pub const SHERPA_MANAGEMENT_IP_INDEX: u32 = 2;
/// Lab Tailscale gateway container host offset.
pub const TAILSCALE_GATEWAY_HOST_OFFSET: u32 = 3;
/// The Tailscale gateway container's name before its lab ID.
pub const TAILSCALE_GATEWAY_NAME: &str = "sherpa-tailnet";
/// Lab management host offsets for nodes and the DHCP range. Node `n` (1-based index) gets
/// the offset `start + n - 1`.
pub const MANAGEMENT_NODE_HOST_OFFSETS: RangeInclusive<u32> = 21..=254;
pub const SHERPA_USERNAME: &str = "sherpa";
pub const ADMIN_USERNAME: &str = "sherpa-admin";
/// User documentation on generating and registering SSH keys, linked from errors.
pub const SSH_KEYS_DOCS_URL: &str = "https://docs.sherpalabs.net/architecture/ssh-keys/";
/// The documentation of the device password, the password of a user's login on lab nodes.
pub const DEVICE_PASSWORD_DOCS_URL: &str =
    "https://docs.sherpalabs.net/architecture/device-password/";
/// Where the Tailscale gateway docs are, for route approval and recovery.
pub const TAILNET_DOCS_URL: &str = "https://docs.sherpalabs.net/architecture/networking/tailscale/";
/// The first user account for unattended `sherpad init`, and the account `sherpa login` uses.
pub const SHERPA_USER_NAME_ENV: &str = "SHERPA_USER_NAME";
pub const SHERPA_USER_PASSWORD_ENV: &str = "SHERPA_USER_PASSWORD";
pub const USERNAME_MIN_LENGTH: usize = 3;
pub const USERNAME_MAX_LENGTH: usize = 32;
/// Names users cannot take: Sherpa's own accounts, privileged and system accounts, and the
/// default users of common guest images, which would collide on lab nodes.
pub const RESERVED_USERNAMES: &[&str] = &[
    // Sherpa and privileged accounts
    "root",
    SHERPA_USERNAME,
    ADMIN_USERNAME,
    "admin",
    "administrator",
    "superuser",
    "sysadmin",
    "toor",
    "operator",
    "wheel",
    // Linux system accounts
    "bin",
    "daemon",
    "sys",
    "sync",
    "games",
    "man",
    "lp",
    "mail",
    "news",
    "uucp",
    "proxy",
    "www-data",
    "backup",
    "list",
    "irc",
    "gnats",
    "nobody",
    "nogroup",
    "messagebus",
    "sshd",
    "polkitd",
    "syslog",
    "_apt",
    // Cloud and distribution default users
    "ubuntu",
    "debian",
    "core",
    "centos",
    "fedora",
    "rocky",
    "almalinux",
    "alpine",
    "freebsd",
    "ec2-user",
    "cloud-user",
    "opc",
    "vagrant",
    "guest",
    // Network OS default and generated accounts
    "enable_1",
    "cisco",
    "cumulus",
    "netadmin",
    "linuxadmin",
];
/// Prefixes of reserved names: Sherpa's own namespace and system accounts.
pub const RESERVED_USERNAME_PREFIXES: &[&str] = &["sherpa-", "systemd-"];
pub const SHERPA_SSH_CONFIG_FILE: &str = "sherpa_ssh_config";
pub const SHERPA_KNOWN_HOSTS_FILE: &str = "sherpa_known_hosts";
/// The lab CA certificate, saved in the lab folder for clients to trust lab nodes.
pub const SHERPA_LAB_CA_FILE: &str = "lab-ca.crt";
pub const SHERPA_SSH_INDEX_FILE: &str = "sherpa_lab_hosts";
pub const SHERPA_SSH_INDEX_HEADER: &str = "# Sherpa lab SSH configs - managed by sherpa CLI";
pub const SHERPA_DOMAIN_NAME: &str = "sherpa.lab.local";
pub const LAB_FILE_NAME: &str = "lab-info.toml";
/// The folder beside a lab's `manifest.toml` holding every file the CLI generates for it;
/// see `util::LabFolder`.
pub const LAB_STATE_DIR: &str = ".sherpa";
pub const BRIDGE_PREFIX: &str = "br";

/// Sherpa's embedded database file.
pub const SHERPA_DB_FILE_PATH: &str = "/opt/sherpa/db/sherpa.sqlite";
/// Opens a database that lives only in memory, for tests.
pub const SHERPA_DB_MEMORY: &str = ":memory:";
/// The database and its backups hold password hashes: only sherpad's user may read them.
pub const SHERPA_DB_FILE_MODE: u32 = 0o600;
/// How long a connection waits for a lock held by another connection.
pub const SHERPA_DB_BUSY_TIMEOUT_SECS: u64 = 5;
/// Generated record keys are 20 characters drawn from lowercase ASCII letters and digits, about
/// 103 bits of randomness. Databases upgraded from earlier releases keep the keys they already
/// hold, so changing the length or alphabet would leave two key shapes in use.
pub const SHERPA_DB_KEY_LENGTH: usize = 20;
pub const SHERPA_DB_KEY_CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
pub const SHERPA_SERVER_WS_PORT: u16 = 3030;
pub const SHERPA_SERVER_HTTP_PORT: u16 = 3031;

pub const QEMU_BIN: &str = "/usr/bin/qemu-system-x86_64";
pub const QEMU_URI: &str = "qemu:///system";
pub const SSH_PORT: u16 = 22;
/// What Sherpa's Windows `sshd_config` adds to the SSH server's identification line with
/// `VersionAddendum`. A Windows node's SSH server starts before cloudbase-init has created the
/// owner's account and written their keys, and only shows this once cloudbase-init has written
/// the config and restarted it, so readiness waits for it.
pub const SHERPA_SSHD_READY_MARKER: &str = "sherpa-ready";
/// Talos Linux API (apid) port, used for readiness and bootstrap.
pub const TALOS_API_PORT: u16 = 50000;
/// Kubernetes version of the generated Talos cluster configs (Talos 1.14 default).
pub const TALOS_KUBERNETES_VERSION: &str = "1.37.1";
/// Kubernetes API port of a Talos cluster.
pub const KUBERNETES_API_PORT: u16 = 6443;
/// Host offsets for Talos cluster shared (VIP) addresses, within the lab control-plane block.
pub const TALOS_VIP_HOST_OFFSETS: RangeInclusive<u32> = 11..=12;
/// Host offsets Cilium clusters hand out to load balancers, within the lab control-plane block
/// after the Talos VIPs. Each Cilium cluster takes a block of
/// `TALOS_LOAD_BALANCER_BLOCK_SIZE` of them, in manifest order.
pub const TALOS_LOAD_BALANCER_HOST_OFFSETS: RangeInclusive<u32> = 13..=20;
/// Load balancer addresses per Cilium cluster: its Gateway, Hubble UI and two spare.
pub const TALOS_LOAD_BALANCER_BLOCK_SIZE: u32 = 4;
/// Boot disk size in GB for Talos Linux nodes; Talos needs at least 10 GB.
pub const TALOS_DEFAULT_BOOT_DISK_SIZE: u16 = 20;
pub const HTTP_PORT: u16 = 8080;
/// Front-panel ports of the `sonic_linux` virtual switch: its `Force10-S6000` hardware
/// SKU has 32, `Ethernet0` to `Ethernet124`, each the next multiple of
/// `SONIC_PORT_NAME_STEP`. The VM maps data interface `ethN` to `Ethernet(4 × (N − 1))`.
pub const SONIC_PORT_COUNT: u8 = 32;
/// The step between `sonic_linux` front-panel port numbers: each port is four lanes.
pub const SONIC_PORT_NAME_STEP: u16 = 4;
pub const KVM_OUI: &str = "52:54:00";

pub const MTU_STD: u16 = 1500;
pub const MTU_JUMBO_INT: u16 = 9216;
pub const MTU_JUMBO_NET: u16 = 9600;
/// Interface indexes tried when creating a veth end into a container's netns, from the host's
/// highest index up, before giving up.
pub const NETNS_VETH_INDEX_ATTEMPTS: u32 = 64;
/// Size of the kernel's `tc_netem_qopt`, which starts netem's options.
pub const NETEM_QOPT_LEN: usize = 24;
/// Netlink attribute that carries a qdisc's options (`TCA_OPTIONS`).
pub const TCA_OPTIONS: u16 = 2;
/// `bpf` filter option attribute that carries the name of the filter's program
/// (`TCA_BPF_NAME`).
pub const TCA_BPF_NAME: u16 = 7;
/// netem option attribute for packet reordering (`TCA_NETEM_REORDER`).
pub const TCA_NETEM_REORDER: u16 = 3;
/// netem option attribute for bit-flip corruption (`TCA_NETEM_CORRUPT`).
pub const TCA_NETEM_CORRUPT: u16 = 4;

pub const ZTP_DIR: &str = "ztp";
pub const TFTP_DIR: &str = "tftp";
pub const NODE_CONFIGS_DIR: &str = "configs";
pub const DNSMASQ_DIR: &str = "dnsmasq";
/// The boot container's HTTP root. The image's HTTP server serves everything under it, so only
/// the lab's HTTP configs folder is mounted there.
pub const BOOT_SERVER_HTTP_ROOT: &str = "/opt/ztp";
/// The boot container's TFTP root, the lab's TFTP folder, outside the HTTP root.
pub const BOOT_SERVER_TFTP_ROOT: &str = "/srv/tftp";
/// The boot container's DHCP lease file, inside the container and outside the HTTP root.
pub const BOOT_SERVER_LEASES_FILE: &str = "/var/lib/misc/dnsmasq.leases";
/// The config disk as `RouterOS` names it; the image imports `config.rsc` and the `sherpa` key
/// from it.
pub const MIKROTIK_CONFIG_DISK: &str = "sata1";

/// cloud-init `sudo` rule for admin logins: every command as any user, without a password.
pub const NODE_CLOUD_INIT_SUDO_RULE: &str = "ALL=(ALL) NOPASSWD:ALL";
/// Per-user file in the guest login's home holding manifest environment variables.
pub const NODE_USER_ENV_FILE: &str = ".sherpa_env";
/// Directory in the guest login's home for env files copied by provisioners; login shells
/// load every file in it.
pub const NODE_USER_ENV_DIR: &str = ".env.d";
/// Default mode of files copied by a provisioner, as octal digits.
pub const PROVISIONER_FILE_DEFAULT_MODE: u32 = 600;
/// Seconds an SSH provisioner waits for each connection to a node to open.
pub const PROVISIONER_SSH_CONNECT_TIMEOUT: u64 = 15;
/// Seconds `sherpa ssh` and SSH provisioners wait for a node reached with no jump host to
/// answer on its SSH port before saying it is not reachable from this machine.
pub const SSH_DIRECT_ROUTE_TIMEOUT: u64 = 10;
/// Seconds between attempts to log in to a node that is not accepting the login yet.
pub const PROVISIONER_SSH_RETRY_INTERVAL: u64 = 5;
/// How many nodes the CLI runs ssh provisioners on at once, unless the manifest's
/// `provision_concurrency` says otherwise.
pub const PROVISION_CONCURRENCY: u8 = 8;
/// Seconds each SFTP request may take while a provisioner copies a file to a node.
pub const PROVISIONER_COPY_TIMEOUT: u64 = 300;
/// Seconds an SSH login may take when it can ask for a passphrase or password, so a person
/// typing is not cut off by the usual 30 s authentication limit.
pub const SSH_PROMPT_AUTH_TIMEOUT: u64 = 300;
/// Default mode of container secret files, as octal digits: readable by any user in the
/// container, which may not run as root.
pub const CONTAINER_SECRET_DEFAULT_MODE: u32 = 444;
/// The Docker runtime, registered by the installer, that runs a container in its own Kata
/// Containers VM on Cloud Hypervisor.
pub const CONTAINER_VM_RUNTIME: &str = "sherpa-kata";
/// The infix of the runc container whose network namespace a VM-run container joins.
pub const CONTAINER_NETNS_HOLDER_INFIX: &str = "netns";
/// Docker endpoint driver option naming a network's interface in the container.
pub const DOCKER_ENDPOINT_IFNAME_OPT: &str = "com.docker.network.endpoint.ifname";
/// Directory in a container where its secret files are mounted, read-only.
pub const CONTAINER_SECRETS_DIR: &str = "/run/secrets";

pub const READINESS_TIMEOUT: u64 = 600;
/// Storage pool refreshes retried while concurrent jobs hold the pool (30s in total).
pub const STORAGE_POOL_REFRESH_ATTEMPTS: u32 = 60;
pub const STORAGE_POOL_REFRESH_RETRY_MS: u64 = 500;
pub const IGNITION_VERSION: &str = "3.3.0";

pub const CONTAINER_DISK_NAME: &str = "disk.img";
/// The endings of a node's disks in the storage pool, after `<node>-<lab_id>`. Destroy deletes
/// only volumes named this way, so a new kind of disk needs its ending here.
pub const LAB_VOLUME_SUFFIXES: [&str; 7] = [
    ".qcow2",
    ".iso",
    ".elf",
    "-hdd.qcow2",
    "-cfg.img",
    "-cfg.ign",
    "-disk.img",
];

pub const CONTAINER_WEBDIR_NAME: &str = "webdir";
pub const CONTAINER_WEBDIR_REPO: &str = "ghcr.io/bwks/webdir";
pub const CONTAINER_WEBDIR_VERSION: &str = "0.1.5";
pub const CONTAINER_DNSMASQ_NAME: &str = "sherpa-router";

pub const CONTAINER_DNSMASQ_REPO: &str = "ghcr.io/bwks/sherpa-router";
pub const CONTAINER_DNSMASQ_VERSION: &str = "latest";

pub const CONTAINER_NOKIA_SRLINUX_REPO: &str = "ghcr.io/nokia/srlinux";

pub const CONTAINER_ARISTA_EOS_REPO: &str = "localrepo/arista_eos";

pub const CONTAINER_CISCO_IOL_REPO: &str = "localrepo/cisco_iol";
pub const CONTAINER_CISCO_IOLL2_REPO: &str = "localrepo/cisco_ioll2";

pub const CONTAINER_JUNIPER_EVOLVED_REPO: &str = "localrepo/juniper_evolved";
/// The memory, in MiB, a cJunosEvolved container needs beyond its EVO VM's: its Docker memory
/// limit is the EVO VM's memory plus this.
pub const JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB: u16 = 2048;

pub const CONTAINER_FRR_REPO: &str = "quay.io/frrouting/frr";

/// The repo of the `sonic_linux` container, the name upstream's `docker-sonic-vs` archive
/// carries. No registry publishes a current image, so it is built and imported.
pub const CONTAINER_SONIC_LINUX_REPO: &str = "docker-sonic-vs";

pub const CONTAINER_GITLAB_CE_REPO: &str = "gitlab/gitlab-ce";

pub const CONTAINER_SURREAL_DB_REPO: &str = "surrealdb/surrealdb";

pub const CONTAINER_MONGO_DB_REPO: &str = "mongo";

pub const CONTAINER_NATS_REPO: &str = "nats";

pub const CONTAINER_FORGEJO_REPO: &str = "codeberg.org/forgejo/forgejo";

pub const CONTAINER_HASHICORP_VAULT_REPO: &str = "hashicorp/vault";
pub const CONTAINER_KEYCLOAK_REPO: &str = "quay.io/keycloak/keycloak";
/// Keycloak client Kubernetes clusters accept tokens for: their audience.
pub const KEYCLOAK_KUBERNETES_CLIENT_ID: &str = "kubernetes";
/// Keycloak group bound to the `cluster-admin` role in clusters.
pub const KEYCLOAK_CLUSTER_ADMINS_GROUP: &str = "cluster-admins";
/// Keycloak group bound to the `view` role in clusters.
pub const KEYCLOAK_CLUSTER_VIEWERS_GROUP: &str = "cluster-viewers";
/// Audience of the service account tokens a cluster's workloads log in to Vault with.
pub const VAULT_KUBERNETES_AUDIENCE: &str = "vault";
/// Vault role a cluster's workloads log in with.
pub const VAULT_KUBERNETES_ROLE: &str = "workload";

// OpenTelemetry defaults
pub const OTEL_DEFAULT_ENDPOINT: &str = "http://localhost:4317";
pub const OTEL_DEFAULT_SERVICE_NAME: &str = "sherpad";
pub const OTEL_DEFAULT_SAMPLE_RATIO: f64 = 1.0;

// Host memory admission
/// Memory in MiB kept back for the host (its OS, sherpad, QEMU's own use and containers without
/// a memory limit) when deciding whether nodes fit on the server.
pub const HOST_MEMORY_DEFAULT_RESERVE_MIB: u32 = 4096;
/// The percentage of the server's memory guests may be given in total. 100 never gives out more
/// memory than the server has.
pub const HOST_MEMORY_DEFAULT_OVERCOMMIT_PERCENT: u32 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    /// Lab management `/24` host offsets reserved for Sherpa infrastructure.
    const MANAGEMENT_INFRA_HOST_OFFSETS: RangeInclusive<u32> = 1..=10;
    /// Lab management host offsets reserved for lab control-plane addresses.
    const MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS: RangeInclusive<u32> = 11..=20;

    #[test]
    fn lab_addressing_007_infrastructure_addresses_are_apart_from_each_other_and_the_nodes() {
        assert_eq!(*MANAGEMENT_INFRA_HOST_OFFSETS.start(), 1);
        assert_eq!(
            *MANAGEMENT_INFRA_HOST_OFFSETS.end() + 1,
            *MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS.start()
        );
        assert_eq!(
            *MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS.end() + 1,
            *MANAGEMENT_NODE_HOST_OFFSETS.start()
        );
        // .255 is the /24 broadcast address.
        assert_eq!(*MANAGEMENT_NODE_HOST_OFFSETS.end(), 254);

        let offsets = [
            MANAGEMENT_GATEWAY_HOST_OFFSET,
            SHERPA_MANAGEMENT_IP_INDEX,
            TAILSCALE_GATEWAY_HOST_OFFSET,
        ];
        for (i, offset) in offsets.iter().enumerate() {
            assert!(MANAGEMENT_INFRA_HOST_OFFSETS.contains(offset));
            assert!(!offsets[i + 1..].contains(offset));
        }
    }

    #[test]
    fn cluster_addresses_006_cluster_addresses_lie_between_the_infrastructure_and_the_nodes() {
        assert!(MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS.contains(TALOS_VIP_HOST_OFFSETS.start()));
        assert!(MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS.contains(TALOS_VIP_HOST_OFFSETS.end()));
        assert_eq!(
            *TALOS_VIP_HOST_OFFSETS.end() + 1,
            *TALOS_LOAD_BALANCER_HOST_OFFSETS.start()
        );
        assert_eq!(
            *TALOS_LOAD_BALANCER_HOST_OFFSETS.end(),
            *MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS.end()
        );
        let count =
            TALOS_LOAD_BALANCER_HOST_OFFSETS.end() - TALOS_LOAD_BALANCER_HOST_OFFSETS.start() + 1;
        assert_eq!(count % TALOS_LOAD_BALANCER_BLOCK_SIZE, 0);
    }
}
