use std::net::Ipv4Addr;
use std::ops::RangeInclusive;

use ipnet::Ipv4Net;

pub const SHERPA_BASE_DIR: &str = "/opt/sherpa";
pub const SHERPA_CONFIG_FILE: &str = "sherpa.toml";
pub const SHERPA_MANIFEST_FILE: &str = "manifest.toml";
pub const SHERPA_LAB_MANIFEST_FILE: &str = "manifest.toml";
pub const SHERPA_CONFIG_DIR: &str = "config";
pub const SHERPA_ENV_DIR: &str = "env";
pub const SHERPA_ENV_FILE: &str = "sherpa.env";
pub const SHERPA_SSH_DIR: &str = "ssh";
pub const SHERPA_IMAGES_DIR: &str = "images";
pub const SHERPA_CONTAINERS_DIR: &str = "containers";
pub const SHERPA_BINS_DIR: &str = "bins";
pub const SHERPA_LABS_DIR: &str = "labs";
pub const SHERPA_RUN_DIR: &str = "run";
pub const SHERPA_LOG_DIR: &str = "logs";
pub const SHERPA_CERTS_DIR: &str = ".certs";
pub const SHERPA_BLANK_DISK_DIR: &str = "blank_disk";
pub const _SHERPA_BLANK_DISK_FAT16: &str = "fat16.img";
pub const SHERPA_BLANK_DISK_FAT32: &str = "fat32.img";
pub const SHERPA_BLANK_DISK_IOSV: &str = "iosv.img";
pub const SHERPA_BLANK_DISK_ISE: &str = "ise.img";
pub const SHERPA_BLANK_DISK_AOSCX: &str = "aos.img";
pub const SHERPA_BLANK_DISK_JUNOS: &str = "junos.img";
pub const SHERPA_BLANK_DISK_SRLINUX: &str = "srlinux.img";
pub const SHERPA_BLANK_DISK_EXT4_500MB: &str = "ext4-500mb.img";
pub const SHERPA_BLANK_DISK_EXT4_1G: &str = "ext4-1g.img";
pub const SHERPA_BLANK_DISK_EXT4_2G: &str = "ext4-2g.img";
pub const SHERPA_BLANK_DISK_EXT4_3G: &str = "ext4-3g.img";
pub const SHERPA_BLANK_DISK_EXT4_4G: &str = "ext4-4g.img";
pub const SHERPA_BLANK_DISK_EXT4_5G: &str = "ext4-5g.img";
pub const VM_DISK_FILENAME: &str = "virtioa.qcow2";
pub const UNIKERNEL_KERNEL_FILENAME: &str = "kernel.elf";
pub const UNIKERNEL_DISK_FILENAME: &str = "disk.qcow2";
// ============================================================================
// Full directory paths
// ============================================================================
pub const SHERPA_CONFIG_PATH: &str = "/opt/sherpa/config";
pub const SHERPA_ENV_PATH: &str = "/opt/sherpa/env";
pub const SHERPA_SSH_PATH: &str = "/opt/sherpa/ssh";
pub const SHERPA_IMAGES_PATH: &str = "/opt/sherpa/images";
pub const SHERPA_CONTAINERS_PATH: &str = "/opt/sherpa/containers";
pub const SHERPA_BINS_PATH: &str = "/opt/sherpa/bins";
pub const SHERPA_LABS_PATH: &str = "/opt/sherpa/labs";
/// Lab directories hold node configs and user data: the `sherpa` user and group only.
pub const SHERPA_LABS_DIR_MODE: u32 = 0o2750;
pub const SHERPA_RUN_PATH: &str = "/opt/sherpa/run";
pub const SHERPA_LOG_PATH: &str = "/opt/sherpa/logs";
pub const SHERPA_CERTS_PATH: &str = "/opt/sherpa/.certs";
pub const SHERPA_BLANK_DISK_PATH: &str = "/opt/sherpa/images/blank_disk";
pub const WEB_ASSETS_DIR: &str = "web/static";
// ============================================================================
// Full file paths
// ============================================================================
pub const SHERPA_CONFIG_FILE_PATH: &str = "/opt/sherpa/config/sherpa.toml";
pub const SHERPA_ENV_FILE_PATH: &str = "/opt/sherpa/env/sherpa.env";
pub const SHERPA_SSH_PUBLIC_KEY_PATH: &str = "/opt/sherpa/ssh/sherpa_ssh_key.pub";
pub const SHERPA_SSH_PRIVATE_KEY_PATH: &str = "/opt/sherpa/ssh/sherpa_ssh_key";
pub const SHERPAD_PID_FILE_PATH: &str = "/opt/sherpa/run/sherpad.pid";
pub const SHERPAD_LOG_FILE_PATH: &str = "/opt/sherpa/logs/sherpad.log";
pub const SHERPA_SERVER_CERT_PATH: &str = "/opt/sherpa/.certs/server.crt";
pub const SHERPA_SERVER_KEY_PATH: &str = "/opt/sherpa/.certs/server.key";

pub const SHERPA_STORAGE_POOL: &str = "sherpa-pool";
pub const SHERPA_STORAGE_POOL_PATH: &str = "/opt/sherpa/libvirt/images";
/// The storage pool holds VM disks and ZTP media: `sherpa` and the QEMU group only.
pub const SHERPA_STORAGE_POOL_DIR_MODE: u32 = 0o2750;
/// Group the libvirt QEMU process runs in, which must reach the storage pool.
pub const SHERPA_STORAGE_POOL_GROUP: &str = "kvm";
/// Mode of storage pool volumes. libvirt hands them to the QEMU user while a VM runs.
pub const STORAGE_VOLUME_MODE: &str = "0600";
/// Mode of volumes QEMU opens itself from a raw argument (Flatcar's Ignition `-fw_cfg`),
/// which libvirt does not hand over. The storage pool directory keeps them private.
pub const STORAGE_VOLUME_QEMU_ARG_MODE: &str = "0644";
pub const SHERPA_MANAGEMENT_NETWORK_NAME: &str = "sherpa-management";
pub const SHERPA_MANAGEMENT_NETWORK_BRIDGE: &str = "sherpa-br0";
pub const SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX: &str = "brm";
/// Default management network. Built with `new_assert`, so an invalid prefix fails the build.
pub const SHERPA_MANAGEMENT_NETWORK_IPV4: Ipv4Net =
    Ipv4Net::new_assert(Ipv4Addr::new(172, 31, 0, 0), 16);
/// Lab management `/24` host offsets reserved for Sherpa infrastructure.
pub const MANAGEMENT_INFRA_HOST_OFFSETS: RangeInclusive<u32> = 1..=10;
/// Lab management gateway (libvirt NAT network) host offset.
pub const MANAGEMENT_GATEWAY_HOST_OFFSET: u32 = 1;
/// Lab router (dnsmasq, boot server) host offset.
pub const SHERPA_MANAGEMENT_IP_INDEX: u32 = 2;
/// Lab Tailscale gateway container host offset.
pub const TAILSCALE_GATEWAY_HOST_OFFSET: u32 = 3;
/// Lab management host offsets reserved for lab control-plane addresses.
pub const MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS: RangeInclusive<u32> = 11..=20;
/// Lab management host offsets for nodes and the DHCP range. Node `n` (1-based index) gets
/// the offset `start + n - 1`.
pub const MANAGEMENT_NODE_HOST_OFFSETS: RangeInclusive<u32> = 21..=254;
pub const SHERPA_ISOLATED_NETWORK_NAME: &str = "sherpa-isolated";
pub const SHERPA_ISOLATED_NETWORK_BRIDGE: &str = "sherpa-br666";
pub const SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX: &str = "bri";
pub const SHERPA_RESERVED_NETWORK_NAME: &str = "sherpa-reserved";
pub const SHERPA_RESERVED_NETWORK_BRIDGE_PREFIX: &str = "brr";
pub const SHERPA_BRIDGE_NETWORK_NAME: &str = "sherpa-bridge";
pub const SHERPA_BRIDGE_NETWORK_BRIDGE: &str = "br-sherpa0";
pub const SHERPA_USERNAME: &str = "sherpa";
pub const ADMIN_USERNAME: &str = "admin";
/// User documentation on generating and registering SSH keys, linked from errors.
pub const SSH_KEYS_DOCS_URL: &str = "https://docs.sherpalabs.net/architecture/ssh-keys/";
pub const SHERPA_ADMIN_PASSWORD_ENV: &str = "SHERPA_ADMIN_PASSWORD";
pub const ADMIN_LAB_START_MESSAGE: &str =
    "the built-in admin account cannot start, resume or redeploy labs; use a personal account";
pub const ADMIN_PROTECTED_MESSAGE: &str =
    "The built-in admin account cannot be deleted, renamed or lose administrator rights";
pub const USERNAME_MIN_LENGTH: usize = 3;
pub const USERNAME_MAX_LENGTH: usize = 32;
/// Names users cannot take: Sherpa's own accounts, privileged and system accounts, and the
/// default users of common guest images, which would collide on lab nodes.
pub const RESERVED_USERNAMES: &[&str] = &[
    // Sherpa and privileged accounts
    "root",
    SHERPA_USERNAME,
    ADMIN_USERNAME,
    "administrator",
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
/// Prefixes of reserved system account names.
pub const RESERVED_USERNAME_PREFIXES: &[&str] = &["systemd-"];
pub const SHERPA_PASSWORD: &str = "Everest1953!";
pub const SHERPA_PASSWORD_HASH: &str = "$6$rounds=4096$amTfvavVzUSS6wQS$4jB1NvmLzRytnUjaVaMkw/JjD99eHj9OL2tLcnccQhV7Rw1rVQp8tZQMu4mi6y8NlwsRSSeEPZq44hVPu4tE7/";
pub const SHERPA_PASSWORD_HASH_MD5: &str = "$1$xXKaRUnh$v5.fg9lEMWQGAqu4DAgaG0";
pub const SHERPA_PASSWORD_HASH_SHA256: &str =
    "$5$mhhcowfq$qKK.CiKWZxuOfadgSyOawx6srN3knX4jRgfa8hZxcB0";
pub const SHERPA_SSH_PUBLIC_KEY_FILE: &str = "sherpa_ssh_key.pub";
pub const SHERPA_SSH_PRIVATE_KEY_FILE: &str = "sherpa_ssh_key";
pub const SHERPA_SSH_CONFIG_FILE: &str = "sherpa_ssh_config";
pub const SHERPA_SSH_INDEX_FILE: &str = "sherpa_lab_hosts";
pub const SHERPA_SSH_INDEX_HEADER: &str = "# Sherpa lab SSH configs - managed by sherpa CLI";
pub const SHERPA_DOMAIN_NAME: &str = "sherpa.lab.local";
pub const LAB_FILE_NAME: &str = "lab-info.toml";
pub const BRIDGE_PREFIX: &str = "br";
pub const VETH_PREFIX: &str = "ve";
pub const TAP_PREFIX: &str = "tp";
pub const CONTAINER_VETH_PREFIX: &str = "cv";

pub const SHERPA_DB_NAME: &str = "sherpa";
pub const SHERPA_DB_NAMESPACE: &str = "sherpa";
pub const SHERPA_DB_USER: &str = "sherpa";
pub const SHERPA_DB_SERVER: &str = "localhost";
pub const SHERPA_DB_PORT: u16 = 8000;
pub const SHERPA_SERVER_WS_PORT: u16 = 3030;
pub const SHERPA_SERVER_HTTP_PORT: u16 = 3031;
pub const SHERPA_SERVER_IPV4: &str = "0.0.0.0";

pub const SHERPA_LOOPBACK_PREFIX: &str = "127.127.0.0/16";

pub const SHERPA_MANAGEMENT_NETWORK_IPV6: &str = "fd00:b00b::/48";
pub const SHERPA_LOOPBACK_PREFIX_IPV6: &str = "fd00:1001::/48";

pub const QEMU_BIN: &str = "/usr/bin/qemu-system-x86_64";
pub const QEMU_URI: &str = "qemu:///system";
pub const _DEFAULT_STORAGE_POOL: &str = "default";
pub const _DEFAULT_STORAGE_POOL_PATH: &str = "/var/lib/libvirt/images";
pub const SSH_PORT: u16 = 22;
pub const SSH_PORT_ALT: u16 = 2222;
/// Talos Linux API (apid) port, used for readiness and bootstrap.
pub const TALOS_API_PORT: u16 = 50000;
/// Talos Linux minor version Sherpa generates cluster machine configs for.
pub const TALOS_SUPPORTED_MINOR_VERSION: &str = "1.14";
/// Kubernetes version of the generated Talos cluster configs (Talos 1.14 default).
pub const TALOS_KUBERNETES_VERSION: &str = "1.37.1";
/// Talos Image Factory base URL for downloading Talos Linux images.
pub const TALOS_IMAGE_FACTORY_URL: &str = "https://factory.talos.dev";
/// Image Factory schematic for vanilla Talos Linux (no system extensions). The factory
/// returns its ID when it is posted to `/schematics`.
pub const TALOS_VANILLA_SCHEMATIC_YAML: &str = "customization: {}\n";
/// Host offsets for Talos cluster shared (VIP) addresses, within the lab control-plane block.
pub const TALOS_VIP_HOST_OFFSETS: RangeInclusive<u32> = 11..=12;
/// Boot disk size in GB for Talos Linux nodes; Talos needs at least 10 GB.
pub const TALOS_DEFAULT_BOOT_DISK_SIZE: u16 = 20;
pub const TELNET_PORT: u16 = 2323;
pub const BASE_PORT: u16 = 10000;
pub const HTTP_PORT: u16 = 8080;
pub const TFTP_PORT: u16 = 69;

pub const KVM_OUI: &str = "52:54:00";
pub const ARISTA_OUI: &str = "02:01:00";
pub const CISCO_IOSXE_OUI: &str = "02:02:00";
pub const CISCO_IOSV_OUI: &str = "02:02:01";
pub const CISCO_NXOS_OUI: &str = "02:02:02";
pub const CISCO_IOSXR_OUI: &str = "02:02:03";
pub const JUNIPER_OUI: &str = "02:03:00";
pub const CUMULUS_OUI: &str = "02:04:00";
pub const ARUBA_OUI: &str = "02:05:00";
pub const BOOT_SERVER_MAC: &str = "02:ff:ff:b0:07:01";

pub const BOOT_SERVER_NAME: &str = "boot01";

pub const CLOUD_INIT_USER_DATA: &str = "user-data";
pub const CLOUD_INIT_META_DATA: &str = "meta-data";
pub const CLOUD_INIT_NETWORK_CONFIG: &str = "network-config";
pub const _USER_SSH_DIR: &str = "~/.ssh";
pub const _USER_SSH_PUBLIC_KEY_FILE: &str = "id_rsa.pub";
pub const TEMP_DIR: &str = ".tmp";

pub const MTU_STD: u16 = 1500;
pub const MTU_JUMBO_INT: u16 = 9216;
pub const MTU_JUMBO_NET: u16 = 9600;

pub const ZTP_DIR: &str = "ztp";
pub const ZTP_ISO: &str = "ztp.iso";
pub const ZTP_JSON: &str = "ztp.json";
pub const TFTP_DIR: &str = "tftp";
pub const NODE_CONFIGS_DIR: &str = "configs";
pub const DNSMASQ_DIR: &str = "dnsmasq";
pub const DNSMASQ_CONFIG_FILE: &str = "dnsmasq.conf";
pub const DNSMASQ_LEASES_FILE: &str = "dnsmasq.leases";
pub const CISCO_ZTP_DIR: &str = "cisco";
pub const CISCO_IOSXE_ZTP_CONFIG: &str = "iosxe_config.txt";
pub const CISCO_IOSV_ZTP_CONFIG: &str = "ios_config.txt";
pub const CISCO_ASAV_ZTP_CONFIG: &str = "day0-config";
pub const CISCO_NXOS_ZTP_CONFIG: &str = "nxos_config.txt";
pub const CISCO_IOSXR_ZTP_CONFIG: &str = "iosxr_config.txt";
pub const CISCO_ISE_ZTP_CONFIG: &str = "ise-ztp.conf";
pub const CISCO_FTDV_ZTP_CONFIG: &str = "day0-config";
pub const CUMULUS_ZTP_DIR: &str = "cumulus";
pub const CUMULUS_ZTP_CONFIG: &str = "cumulus-config.txt";
pub const CUMULUS_ZTP: &str = "cumulus-ztp";
pub const ARISTA_ZTP_DIR: &str = "arista";
pub const ARISTA_ABOOT_DIR: &str = "aboot_iso";
pub const ARISTA_VEOS_ZTP_SCRIPT: &str = "veos-ztp.sh";
pub const ARISTA_VEOS_ZTP: &str = "startup-config";
pub const ARISTA_CEOS_ZTP_VOLUME_MOUNT: &str = "/mnt/flash/startup-config";
pub const NOKIA_SRLINUX_ZTP_VOLUME_MOUNT: &str = "/etc/opt/srlinux/config.json";
pub const ARUBA_ZTP_DIR: &str = "aruba";
pub const ARUBA_ZTP_CONFIG: &str = "aos-config.txt";
pub const ARUBA_ZTP_SCRIPT: &str = "aos-config.sh";
pub const JUNIPER_ZTP_DIR: &str = "juniper";
pub const JUNIPER_ZTP_CONFIG: &str = "juniper.conf";
pub const JUNIPER_ZTP_SCRIPT: &str = "junos-ztp.sh";
pub const JUNIPER_ZTP_CONFIG_TGZ: &str = "vmm-config.tgz";
pub const MIKROTIK_CHR_ZTP_CONFIG: &str = "config.rsc";
pub const PALOALTO_ZTP_CONFIG: &str = "init-cfg.txt";
pub const PALOALTO_BOOTSTRAP_CONFIG: &str = "bootstrap.xml";

pub const LAB_CERTS_DIR: &str = "certs";
pub const LAB_CA_CERT_FILE: &str = "ca.crt";
pub const LAB_CA_KEY_FILE: &str = "ca.key";
pub const NODE_CERT_FILE: &str = "node.crt";
pub const NODE_KEY_FILE: &str = "node.key";
pub const NODE_CERTS_DIR: &str = "/etc/sherpa/certs";
pub const NODE_CERTS_DIR_WINDOWS: &str = r"C:\sherpa\certs";
pub const NODE_USER_SCRIPTS_DIR: &str = "/opt/sherpa/user_scripts";
pub const NODE_STARTUP_SCRIPTS_DIR: &str = "/opt/sherpa/startup_scripts";
/// Drop-in directory for per-user doas rules (Alpine).
pub const NODE_DOAS_CONF_DIR: &str = "/etc/doas.d";
/// doas configuration file (OpenBSD, whose doas has no drop-in directory).
pub const NODE_DOAS_CONF_FILE: &str = "/etc/doas.conf";
/// cloud-init `sudo` rule for admin logins: every command as any user, without a password.
pub const NODE_CLOUD_INIT_SUDO_RULE: &str = "ALL=(ALL) NOPASSWD:ALL";
/// Shadow password value that matches no password but, unlike `!`, does not lock the
/// account, so sshd builds without PAM still accept key logins.
pub const NODE_NO_PASSWORD_HASH: &str = "*";
/// Per-user file in the guest login's home holding manifest environment variables.
pub const NODE_USER_ENV_FILE: &str = ".sherpa_env";
/// Directory in the guest login's home for env files copied by provisioners; login shells
/// load every file in it.
pub const NODE_USER_ENV_DIR: &str = ".env.d";
/// Default mode of files copied by a provisioner, as octal digits.
pub const PROVISIONER_FILE_DEFAULT_MODE: u32 = 600;
/// Seconds an SSH provisioner waits for each connection to a node to open.
pub const PROVISIONER_SSH_CONNECT_TIMEOUT: u64 = 15;
/// Seconds between attempts to log in to a node that is not accepting the login yet.
pub const PROVISIONER_SSH_RETRY_INTERVAL: u64 = 5;
/// Default mode of container secret files, as octal digits: readable by any user in the
/// container, which may not run as root.
pub const CONTAINER_SECRET_DEFAULT_MODE: u32 = 444;
/// Docker endpoint driver option naming a network's interface in the container.
pub const DOCKER_ENDPOINT_IFNAME_OPT: &str = "com.docker.network.endpoint.ifname";
/// Directory in a container where its secret files are mounted, read-only.
pub const CONTAINER_SECRETS_DIR: &str = "/run/secrets";
/// RAM-backed server directory holding container secret files, under the daemon's
/// systemd `RuntimeDirectory` (`/run/sherpa`, mode 0700).
pub const SHERPA_SECRETS_DIR: &str = "/run/sherpa/secrets";
/// Root-only directory where cloud-init stages files until their owners exist.
pub const NODE_STAGING_DIR: &str = "/opt/sherpa/staging";
/// Guest account database, read at boot to find a user's home directory.
pub const NODE_PASSWD_FILE: &str = "/etc/passwd";
pub const LAB_CERT_VALIDITY_DAYS: u32 = 365;

pub const READINESS_TIMEOUT: u64 = 600;
/// Seconds `down` waits for a VM to shut down cleanly before powering it off.
pub const VM_SHUTDOWN_TIMEOUT: u64 = 60;
/// Seconds between VM power state checks while waiting for a clean shutdown.
pub const VM_SHUTDOWN_POLL: u64 = 1;
/// Storage pool refreshes retried while concurrent jobs hold the pool (30s in total).
pub const STORAGE_POOL_REFRESH_ATTEMPTS: u32 = 60;
pub const STORAGE_POOL_REFRESH_RETRY_MS: u64 = 500;
/// Attempts to reserve lab subnets when concurrent labs take the chosen ones first.
pub const LAB_SUBNET_ALLOCATION_ATTEMPTS: u32 = 8;
pub const READINESS_SLEEP: u64 = 10;
pub const IGNITION_VERSION: &str = "3.3.0";

pub const DHCP_URI_DIR: &str = "dnsmasq";
pub const DHCP_LEASES_FILE: &str = "dnsmasq.leases";

pub const DOCKER_COMPOSE_VERSION: &str = "2.34.0";

pub const CONTAINER_IMAGE_NAME: &str = "image.tar.gz";
pub const CONTAINER_DISK_NAME: &str = "disk.img";

pub const CONTAINER_WEBDIR_NAME: &str = "webdir";
pub const CONTAINER_WEBDIR_REPO: &str = "ghcr.io/bwks/webdir";
pub const CONTAINER_WEBDIR_VERSION: &str = "0.1.5";
pub const CONTAINER_DNSMASQ_NAME: &str = "sherpa-router";

pub const CONTAINER_DNSMASQ_REPO: &str = "ghcr.io/bwks/sherpa-router";
pub const CONTAINER_DNSMASQ_VERSION: &str = "latest";
pub const CONTAINER_DNSMASQ_CAPABILITIES: &[&str] = &["NET_ADMIN"];

pub const CONTAINER_SRLINUX_NAME: &str = "srlinux";
pub const CONTAINER_NOKIA_SRLINUX_REPO: &str = "ghcr.io/nokia/srlinux";
pub const CONTAINER_NOKIA_SRLINUX_ENV_VARS: &[&str] = &["SRLINUX=1"];
pub const CONTAINER_NOKIA_SRLINUX_USER: &str = "0:0";
pub const CONTAINER_NOKIA_SRLINUX_COMMANDS: &[&str] = &["bash", "/opt/srlinux/bin/sr_linux"];

pub const CONTAINER_ARISTA_CEOS_REPO: &str = "localrepo/arista_ceos";
pub const CONTAINER_ARISTA_CEOS_ENV_VARS: &[&str] = &[
    "INTFTYPE=eth",
    "ETBA=1",
    "SKIP_ZEROTOUCH_BARRIER_IN_SYSDBINIT=1",
    "CEOS=1",
    "EOS_PLATFORM=ceoslab",
    "MAPETH0:1",
    "MGMT_INTF:eth0",
];
pub const CONTAINER_ARISTA_CEOS_COMMANDS: &[&str] = &[
    "/sbin/init",
    "systemd.setenv=INTFTYPE=eth",
    "systemd.setenv=ETBA=1",
    "systemd.setenv=SKIP_ZEROTOUCH_BARRIER_IN_SYSDBINIT=1",
    "systemd.setenv=CEOS=1",
    "systemd.setenv=EOS_PLATFORM=ceoslab",
    "systemd.setenv=container=docker",
    "systemd.setenv=MAPETH0=1",
    "systemd.setenv=MGMT_INTF=eth0",
];

pub const CONTAINER_FRR_REPO: &str = "quay.io/frrouting/frr";
pub const CONTAINER_FRR_ENV_VARS: &[&str] = &[];
pub const CONTAINER_FRR_COMMANDS: &[&str] = &[];
pub const FRR_ZTP_DAEMONS_MOUNT: &str = "/etc/frr/daemons";
pub const FRR_ZTP_CONFIG_MOUNT: &str = "/etc/frr/frr.conf";
pub const FRR_ZTP_STARTUP_MOUNT: &str = "/tmp/sherpa-init.sh";

pub const CONTAINER_GITLAB_CE_REPO: &str = "gitlab/gitlab-ce";
pub const CONTAINER_GITLAB_CE_ENV_VARS: &[&str] = &[];
pub const CONTAINER_GITLAB_CE_COMMANDS: &[&str] = &[];
pub const CONTAINER_GITLAB_CE_SHM_SIZE: i64 = 256;

pub const CONTAINER_SURREAL_DB_REPO: &str = "surrealdb/surrealdb";
pub const CONTAINER_SURREAL_DB_ENV_VARS: &[&str] = &[];
pub const CONTAINER_SURREAL_DB_COMMANDS: &[&str] = &[
    "start",
    "--log",
    "trace",
    "--user",
    SHERPA_USERNAME,
    "--pass",
    SHERPA_PASSWORD,
    "memory",
];

pub const CONTAINER_MONGO_DB_REPO: &str = "mongo";
pub const CONTAINER_MONGO_DB_ENV_VARS: &[&str] = &[];
pub const CONTAINER_MONGO_DB_COMMANDS: &[&str] = &[];

pub const CONTAINER_NATS_REPO: &str = "nats";
pub const CONTAINER_NATS_ENV_VARS: &[&str] = &[];
pub const CONTAINER_NATS_COMMANDS: &[&str] = &[];

pub const CONTAINER_FORGEJO_REPO: &str = "codeberg.org/forgejo/forgejo";
pub const CONTAINER_FORGEJO_ENV_VARS: &[&str] = &[];
pub const CONTAINER_FORGEJO_COMMANDS: &[&str] = &[];

pub const CONTAINER_HASHICORP_VAULT_REPO: &str = "hashicorp/vault";
pub const CONTAINER_HASHICORP_VAULT_ENV_VARS: &[&str] = &["VAULT_ADDR=http://127.0.0.1:8200"];
pub const CONTAINER_HASHICORP_VAULT_COMMANDS: &[&str] = &[
    "sh",
    "-c",
    "mkdir -p /vault/data && exec vault server -config=/vault/config/config.hcl",
];
pub const CONTAINER_HASHICORP_VAULT_CAPABILITIES: &[&str] = &["IPC_LOCK"];
pub const VAULT_ZTP_CONFIG_MOUNT: &str = "/vault/config/config.hcl";

// OpenTelemetry defaults
pub const OTEL_DEFAULT_ENDPOINT: &str = "http://localhost:4317";
pub const OTEL_DEFAULT_PROTOCOL: &str = "grpc";
pub const OTEL_DEFAULT_SERVICE_NAME: &str = "sherpad";
pub const OTEL_DEFAULT_SAMPLE_RATIO: f64 = 1.0;

// OpenTelemetry metric names
pub const OTEL_METRIC_WS_CONNECTIONS: &str = "sherpad.ws.connections";
pub const OTEL_METRIC_RPC_DURATION: &str = "sherpad.rpc.duration";
pub const OTEL_METRIC_OPERATION_DURATION: &str = "sherpad.operation.duration";
pub const OTEL_METRIC_ERROR_COUNT: &str = "sherpad.errors";
pub const OTEL_METRIC_EXPORT_INTERVAL_SECS: u64 = 60;

// Sherpad daemon constants
pub const SHERPAD_PID_FILE: &str = "sherpad.pid";
pub const SHERPAD_LOG_FILE: &str = "sherpad.log";
pub const SHERPAD_HOST: &str = "127.0.0.1";
pub const SHERPAD_PORT: u16 = 3030;

// JWT Authentication constants
pub const JWT_SECRET_PATH: &str = "/opt/sherpa/.secret/jwt.secret";
pub const JWT_TOKEN_EXPIRY_SECONDS: i64 = 604_800; // 7 days

// TLS certificate paths
pub const SHERPA_SERVER_CERT_FILE: &str = "server.crt";
pub const SHERPA_SERVER_KEY_FILE: &str = "server.key";
pub const SHERPA_CA_CERT_FILE: &str = "ca.crt";

// ============================================================================
// JSON-RPC Error Messages
// ============================================================================

// Standard JSON-RPC error messages
pub const RPC_MSG_PARSE_ERROR: &str = "Parse error";
pub const RPC_MSG_INVALID_REQUEST: &str = "Invalid Request";
pub const RPC_MSG_INTERNAL_ERROR: &str = "Internal error";

// Authentication and authorization messages
pub const RPC_MSG_AUTH_REQUIRED: &str = "Authentication required";
pub const RPC_MSG_AUTH_INVALID: &str = "Invalid username or password";
pub const RPC_MSG_AUTH_ERROR: &str = "Authentication error";
pub const RPC_MSG_TOKEN_CREATE_FAILED: &str = "Failed to create authentication token";

// Web UI
/// Element on the admin users page that shows a refused user delete (`hx-retarget` value).
pub const ADMIN_USER_DELETE_ERROR_TARGET: &str = "#user-delete-error";

// Authorization messages
pub const RPC_MSG_ACCESS_DENIED_OWN_PASSWORD: &str =
    "Access denied: you can only change your own password";
pub const RPC_MSG_ACCESS_DENIED_OWN_INFO: &str =
    "Access denied: you can only view your own information";

// User management - admin-only operations
pub const RPC_MSG_USER_ADMIN_ONLY_CREATE: &str =
    "Access denied: only administrators can create users";
pub const RPC_MSG_USER_ADMIN_ONLY_LIST: &str = "Access denied: only administrators can list users";
pub const RPC_MSG_USER_ADMIN_ONLY_DELETE: &str =
    "Access denied: only administrators can delete users";

// User management - operation failures
pub const RPC_MSG_USER_CREATE_FAILED: &str = "Failed to create user";
pub const RPC_MSG_USER_LIST_FAILED: &str = "Failed to list users";
pub const RPC_MSG_USER_DELETE_FAILED: &str = "Failed to delete user";
pub const RPC_MSG_USER_PASSWORD_UPDATE_FAILED: &str = "Failed to update password";
pub const RPC_MSG_SSH_KEY_ADD_FAILED: &str = "Failed to add SSH key";
pub const RPC_MSG_SSH_KEY_REMOVE_FAILED: &str = "Failed to remove SSH key";
pub const RPC_MSG_PASSWORD_VALIDATION_FAILED: &str = "Password validation failed";

// Lab operations
pub const RPC_MSG_LAB_INSPECT_FAILED: &str = "Inspect operation failed";
pub const RPC_MSG_LAB_DESTROY_FAILED: &str = "Destroy operation failed";
pub const RPC_MSG_LAB_CLEAN_FAILED: &str = "Clean operation failed";
pub const RPC_MSG_LAB_UP_FAILED: &str = "Up operation failed";
pub const RPC_MSG_LAB_DOWN_FAILED: &str = "Down operation failed";
pub const RPC_MSG_LAB_CANCEL_FAILED: &str = "Stopping the lab's operations failed";
pub const RPC_MSG_LAB_RESUME_FAILED: &str = "Resume operation failed";
pub const RPC_MSG_NODE_SECRETS_FAILED: &str = "Writing the node's secret files failed";

// Admin-only operations
pub const RPC_MSG_ADMIN_ONLY_CLEAN: &str =
    "Access denied: only administrators can run clean operations";

// Image operations
pub const RPC_MSG_IMAGE_IMPORT_FAILED: &str = "Image import operation failed";
pub const RPC_MSG_IMAGE_LIST_FAILED: &str = "Image list operation failed";
pub const RPC_MSG_IMAGE_SHOW_FAILED: &str = "Image show operation failed";
pub const RPC_MSG_INVALID_PARAMS_IMAGE_SHOW: &str = "Invalid params: expected ShowImageRequest";
pub const RPC_MSG_IMAGE_SCAN_FAILED: &str = "Image scan operation failed";
pub const RPC_MSG_ADMIN_ONLY_IMAGE_IMPORT: &str =
    "Access denied: only administrators can import images";
pub const RPC_MSG_ADMIN_ONLY_IMAGE_SCAN: &str =
    "Access denied: only administrators can scan images";
pub const RPC_MSG_IMAGE_DELETE_FAILED: &str = "Image delete operation failed";
pub const RPC_MSG_ADMIN_ONLY_IMAGE_DELETE: &str =
    "Access denied: only administrators can delete images";
pub const RPC_MSG_INVALID_PARAMS_IMAGE_DELETE: &str = "Invalid params: expected DeleteImageRequest";
pub const RPC_MSG_IMAGE_SET_DEFAULT_FAILED: &str = "Image set-default operation failed";
pub const RPC_MSG_ADMIN_ONLY_IMAGE_SET_DEFAULT: &str =
    "Access denied: only administrators can set the default image";
pub const RPC_MSG_INVALID_PARAMS_IMAGE_SET_DEFAULT: &str =
    "Invalid params: expected SetDefaultImageRequest";

// Serialization errors
pub const RPC_MSG_SERIALIZE_FAILED: &str = "Failed to serialize response";

// Container pull operations
pub const RPC_MSG_CONTAINER_PULL_FAILED: &str = "Container image pull operation failed";
pub const RPC_MSG_ADMIN_ONLY_CONTAINER_PULL: &str =
    "Access denied: only administrators can pull container images";
pub const RPC_MSG_INVALID_PARAMS_CONTAINER_PULL: &str =
    "Invalid params: expected ContainerPullRequest";

// VM image download operations
pub const RPC_MSG_IMAGE_DOWNLOAD_FAILED: &str = "Image download operation failed";
pub const RPC_MSG_ADMIN_ONLY_IMAGE_DOWNLOAD: &str =
    "Access denied: only administrators can download images";
pub const RPC_MSG_INVALID_PARAMS_IMAGE_DOWNLOAD: &str =
    "Invalid params: expected DownloadImageRequest";

// Node name params
pub const RPC_MSG_INVALID_PARAMS_NODE_NAME: &str = "Invalid 'node_name' parameter";

// Impairment operations
pub const RPC_MSG_IMPAIRMENT_UPDATE_FAILED: &str = "Link impairment update failed";
pub const RPC_MSG_INVALID_PARAMS_IMPAIRMENT: &str =
    "Invalid params: expected lab_id, link_index, and token";

// Redeploy operations
pub const RPC_MSG_REDEPLOY_FAILED: &str = "Redeploy operation failed";
pub const RPC_MSG_INVALID_PARAMS_REDEPLOY: &str =
    "Invalid params: expected lab_id, node_name, manifest, and token";

// Invalid params messages
pub const RPC_MSG_INVALID_PARAMS_LAB_ID: &str = "Invalid params: 'lab_id' (string) is required";
pub const RPC_MSG_INVALID_PARAMS_NODE_SECRETS: &str =
    "Invalid params: expected lab_id, node_name, files, and token";
pub const RPC_MSG_INVALID_PARAMS_MANIFEST: &str = "Invalid params: 'manifest' (object) is required";
pub const RPC_MSG_INVALID_PARAMS_LOGIN: &str =
    "Invalid params: expected {username: string, password: string}";
pub const RPC_MSG_INVALID_PARAMS_TOKEN: &str = "Invalid params: expected {token: string}";
pub const RPC_MSG_INVALID_PARAMS_CREATE_USER: &str = "Invalid params: expected CreateUserRequest";
pub const RPC_MSG_INVALID_PARAMS_DELETE_USER: &str = "Invalid params: expected DeleteUserRequest";
pub const RPC_MSG_INVALID_PARAMS_CHANGE_PASSWORD: &str =
    "Invalid params: expected ChangePasswordRequest";
pub const RPC_MSG_INVALID_PARAMS_GET_USER_INFO: &str =
    "Invalid params: expected GetUserInfoRequest";
pub const RPC_MSG_INVALID_PARAMS_ADD_SSH_KEY: &str = "Invalid params: expected AddSshKeyRequest";
pub const RPC_MSG_INVALID_PARAMS_REMOVE_SSH_KEY: &str =
    "Invalid params: expected RemoveSshKeyRequest";
pub const RPC_MSG_INVALID_PARAMS_IMPORT: &str = "Invalid params: expected ImportRequest";
pub const RPC_MSG_INVALID_PARAMS_IMAGE_LIST: &str = "Invalid params: expected ListImagesRequest";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_management_network_ipv4_is_172_31_0_0_16() {
        assert_eq!(SHERPA_MANAGEMENT_NETWORK_IPV4.to_string(), "172.31.0.0/16");
        assert_eq!(SHERPA_MANAGEMENT_NETWORK_IPV4.prefix_len(), 16);
    }

    #[test]
    fn test_management_host_offset_blocks_are_ordered_and_disjoint() {
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
    }

    #[test]
    fn test_infrastructure_addresses_are_distinct_and_in_the_infra_block() {
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
    fn test_talos_vips_are_in_the_control_plane_block() {
        assert!(MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS.contains(TALOS_VIP_HOST_OFFSETS.start()));
        assert!(MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS.contains(TALOS_VIP_HOST_OFFSETS.end()));
    }
}
