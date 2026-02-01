pub const SHERPA_BASE_DIR: &str = "/opt/sherpa";
pub const SHERPA_CONFIG_FILE: &str = "sherpa.toml";
pub const SHERPA_MANIFEST_FILE: &str = "manifest.toml";
pub const SHERPA_CONFIG_DIR: &str = "config";
pub const SHERPA_SSH_DIR: &str = "ssh";
pub const SHERPA_IMAGES_DIR: &str = "images";
pub const SHERPA_CONTAINERS_DIR: &str = "containers";
pub const SHERPA_BINS_DIR: &str = "bins";
pub const SHERPA_LABS_DIR: &str = "labs";
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
pub const SHERPA_STORAGE_POOL: &str = "sherpa-pool";
pub const SHERPA_STORAGE_POOL_PATH: &str = "/opt/sherpa/libvirt/images";
pub const SHERPA_MANAGEMENT_NETWORK_NAME: &str = "sherpa-management";
pub const SHERPA_MANAGEMENT_NETWORK_BRIDGE: &str = "sherpa-br0";
pub const SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX: &str = "brsm";
pub const SHERPA_MANAGEMENT_NETWORK_IPV4: &str = "172.31.0.0/16";
pub const SHERPA_MANAGEMENT_IP_INDEX: u32 = 2;
pub const SHERPA_ISOLATED_NETWORK_NAME: &str = "sherpa-blackhole";
pub const SHERPA_ISOLATED_NETWORK_BRIDGE: &str = "sherpa-br666";
pub const SHERPA_ISOLATED_NETWORK_BRIDGE_PREFIX: &str = "bri";
pub const SHERPA_BRIDGE_NETWORK_NAME: &str = "sherpa-bridge";
pub const SHERPA_BRIDGE_NETWORK_BRIDGE: &str = "br-sherpa0";
pub const SHERPA_USERNAME: &str = "sherpa";
pub const SHERPA_PASSWORD: &str = "Everest1953!";
pub const SHERPA_PASSWORD_HASH: &str = "$6$rounds=4096$amTfvavVzUSS6wQS$4jB1NvmLzRytnUjaVaMkw/JjD99eHj9OL2tLcnccQhV7Rw1rVQp8tZQMu4mi6y8NlwsRSSeEPZq44hVPu4tE7/";
pub const SHERPA_SSH_PUBLIC_KEY_FILE: &str = "sherpa_ssh_key.pub";
pub const SHERPA_SSH_PRIVATE_KEY_FILE: &str = "sherpa_ssh_key";
pub const SHERPA_SSH_CONFIG_FILE: &str = "sherpa_ssh_config";
pub const SHERPA_DOMAIN_NAME: &str = "sherpa.lab.local";
pub const LAB_FILE_NAME: &str = "lab-info.toml";
pub const BRIDGE_PREFIX: &str = "br";
pub const VETH_PREFIX: &str = "ve";

pub const SHERPA_DB_NAME: &str = "sherpa";
pub const SHERPA_DB_NAMESPACE: &str = "sherpa";
pub const SHERPA_DB_SERVER: &str = "localhost";
pub const SHERPA_DB_PORT: u16 = 8000;

pub const QEMU_BIN: &str = "/usr/bin/qemu-system-x86_64";
pub const QEMU_URI: &str = "qemu:///system";
pub const _DEFAULT_STORAGE_POOL: &str = "default";
pub const _DEFAULT_STORAGE_POOL_PATH: &str = "/var/lib/libvirt/images";
pub const SSH_PORT: u16 = 22;
pub const SSH_PORT_ALT: u16 = 2222;
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
pub const ARISTA_VEOS_ZTP_SCRIPT: &str = "veos-ztp.sh";
pub const ARISTA_VEOS_ZTP: &str = "startup-config";
pub const ARISTA_CEOS_ZTP_VOLUME_MOUNT: &str = "/mnt/flash/startup-config";
pub const ARUBA_ZTP_DIR: &str = "aruba";
pub const ARUBA_ZTP_CONFIG: &str = "aos-config.txt";
pub const ARUBA_ZTP_SCRIPT: &str = "aos-config.sh";
pub const JUNIPER_ZTP_DIR: &str = "juniper";
pub const JUNIPER_ZTP_CONFIG: &str = "juniper.conf";
pub const JUNIPER_ZTP_SCRIPT: &str = "junos-ztp.sh";
pub const JUNIPER_ZTP_CONFIG_TGZ: &str = "vmm-config.tgz";

pub const READINESS_TIMEOUT: u64 = 600;
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
pub const CONTAINER_SRLINUX_NAME: &str = "srlinux";

pub const CONTAINER_NOKIA_SRLINUX_REPO: &str = "ghcr.io/nokia/srlinux";
pub const CONTAINER_NOKIA_SRLINUX_ENV_VARS: &[&str] = &["SRLINUX=1"];
pub const CONTAINER_NOKIA_SRLINUX_COMMANDS: &[&str] =
    &["sudo", "bash", "/opt/srlinux/bin/sr_linux"];

pub const CONTAINER_ARISTA_CEOS_REPO: &str = "arista/ceos";
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
