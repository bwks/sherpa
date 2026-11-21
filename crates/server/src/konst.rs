//! Constants used by more than one service or module tree of the server. A constant used by one
//! module is private in that module; one used by a service and its submodules, or by one module
//! tree such as `daemon`, lives in that service or tree.

pub(crate) const SHERPA_BLANK_DISK_DIR: &str = "blank_disk";
pub(crate) const SHERPA_CONFIG_FILE_PATH: &str = "/opt/sherpa/config/sherpa.toml";
pub(crate) const SHERPA_SERVER_CERT_PATH: &str = "/opt/sherpa/.certs/server.crt";
pub(crate) const SHERPA_MANAGEMENT_NETWORK_NAME: &str = "sherpa-management";
pub(crate) const SHERPA_ADMIN_PASSWORD_ENV: &str = "SHERPA_ADMIN_PASSWORD";
pub(crate) const TAP_PREFIX: &str = "tp";
/// Host side of a P2p container's private bridge member veth; it joins the bridge.
pub(crate) const BRIDGE_MEMBER_HOST_VETH_PREFIX: &str = "cb";
/// Prefix of the host end of a veth created into a P2p container's network namespace.
pub(crate) const CONTAINER_HOST_VETH_PREFIX: &str = "cd";
/// Directory of Sherpa's embedded database and its backups.
pub(crate) const SHERPA_DB_PATH: &str = "/opt/sherpa/db";
/// Where database backups taken by the server are written.
pub(crate) const SHERPA_DB_BACKUP_PATH: &str = "/opt/sherpa/db/backups";
/// The backup directory: `sherpa` only. A backup is written before its file is restricted.
pub(crate) const SHERPA_DB_BACKUP_DIR_MODE: u32 = 0o700;
pub(crate) const SHERPA_SERVER_IPV4: &str = "0.0.0.0";
pub(crate) const SHERPA_MANAGEMENT_NETWORK_IPV6: &str = "fd00:b00b::/48";
pub(crate) const ZTP_ISO: &str = "ztp.iso";
pub(crate) const DNSMASQ_CONFIG_FILE: &str = "dnsmasq.conf";
/// The mode of a config Sherpa renders for a node: only `sherpa` reads it. The processes that
/// read it as well run as root: Docker for a bind mount and the container's own init, sherpad
/// when it uploads the boot media, and the boot container's HTTP server.
pub(crate) const NODE_CONFIG_FILE_MODE: u32 = 0o600;
/// The mode of a folder holding node configs: a VM's boot folder, a container's node folder,
/// the lab's `ztp` folder and its HTTP configs folder.
pub(crate) const NODE_CONFIG_DIR_MODE: u32 = 0o700;
/// The mode of a node's TFTP config. dnsmasq serves only files others can read: as root, as it
/// runs in the boot container, it serves world-readable files only, and as its own `dnsmasq`
/// user, which it drops to when started otherwise, it owns nothing on the host. The group, which
/// on the host is the people in `sherpa`, gets nothing, and on the host others cannot reach the
/// lab's folder.
pub(crate) const TFTP_CONFIG_FILE_MODE: u32 = 0o604;
/// The mode of the lab's TFTP folder. Root could read a `0700` folder, but dnsmasq as its own
/// user refuses a TFTP root it cannot list, so others get read and search: on the host they
/// cannot reach the lab's folder, and the folder keeps working whichever user dnsmasq runs as.
pub(crate) const TFTP_DIR_MODE: u32 = 0o705;
pub(crate) const LAB_CERTS_DIR: &str = "certs";
pub(crate) const LAB_CA_CERT_FILE: &str = "ca.crt";
pub(crate) const LAB_CA_KEY_FILE: &str = "ca.key";
/// Login shell on NixOS, which has no `/bin/bash`: its sshd refuses an account whose shell does
/// not exist.
pub(crate) const NODE_NIXOS_SHELL: &str = "/run/current-system/sw/bin/bash";
/// RAM-backed server directory holding container secret files, under the daemon's
/// systemd `RuntimeDirectory` (`/run/sherpa`, mode 0700).
pub(crate) const SHERPA_SECRETS_DIR: &str = "/run/sherpa/secrets";
/// Folder of the labs' serial console sockets, one folder per lab below it. On disk, not under
/// `/run`, so a domain defined with its socket here still starts after the server reboots.
pub(crate) const SHERPA_CONSOLE_DIR: &str = "/opt/sherpa/run/console";
/// Group of the console folders. libvirt binds the serial sockets as root; the QEMU user
/// (`libvirt-qemu`) is in it, and so is `sherpa`, which connects to them.
pub(crate) const CONSOLE_GROUP: &str = "kvm";
/// Mode of [`SHERPA_CONSOLE_DIR`]: no one outside `sherpa` and [`CONSOLE_GROUP`] can list it.
pub(crate) const CONSOLE_DIR_MODE: u32 = 0o2750;
/// Mode of a lab's console folder, `sherpa:kvm`: no one outside the two can reach a socket in it.
pub(crate) const CONSOLE_LAB_DIR_MODE: u32 = 0o2770;
/// The longest path a UNIX socket can have: `sun_path` holds 108 bytes, the last a NUL.
pub(crate) const UNIX_SOCKET_PATH_MAX: usize = 107;
/// Seconds between checks that the login session behind an open console is still valid.
pub(crate) const CONSOLE_SESSION_CHECK_SECS: u64 = 30;
pub(crate) const READINESS_SLEEP: u64 = 10;
/// Keycloak HTTPS port.
pub(crate) const KEYCLOAK_HTTPS_PORT: u16 = 8443;
/// Realm Sherpa makes on Keycloak nodes; Kubernetes clusters accept its users.
pub(crate) const KEYCLOAK_REALM: &str = "sherpa";
/// Vault API (HTTPS) and cluster ports.
pub(crate) const VAULT_API_PORT: u16 = 8200;
pub(crate) const JWT_TOKEN_EXPIRY_SECONDS: i64 = 604_800; // 7 days
