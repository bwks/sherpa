//! Constants used by more than one module of the server. A constant used by one module is
//! private in that module.

pub(crate) const SHERPA_BLANK_DISK_DIR: &str = "blank_disk";
pub(crate) const SHERPA_CONFIG_FILE_PATH: &str = "/opt/sherpa/config/sherpa.toml";
pub(crate) const SHERPAD_PID_FILE_PATH: &str = "/opt/sherpa/run/sherpad.pid";
pub(crate) const SHERPAD_LOG_FILE_PATH: &str = "/opt/sherpa/logs/sherpad.log";
pub(crate) const SHERPA_SERVER_CERT_PATH: &str = "/opt/sherpa/.certs/server.crt";
pub(crate) const SHERPA_MANAGEMENT_NETWORK_NAME: &str = "sherpa-management";
pub(crate) const SHERPA_ADMIN_PASSWORD_ENV: &str = "SHERPA_ADMIN_PASSWORD";
pub(crate) const SHERPA_PASSWORD_HASH: &str = "$6$rounds=4096$amTfvavVzUSS6wQS$4jB1NvmLzRytnUjaVaMkw/JjD99eHj9OL2tLcnccQhV7Rw1rVQp8tZQMu4mi6y8NlwsRSSeEPZq44hVPu4tE7/";
/// The public half of `SHERPA_SSH_PRIVATE_KEY_FILE`, in a lab folder.
pub(crate) const SHERPA_SSH_PUBLIC_KEY_FILE: &str = "sherpa_ssh_key.pub";
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
pub(crate) const LAB_CERTS_DIR: &str = "certs";
pub(crate) const LAB_CA_CERT_FILE: &str = "ca.crt";
pub(crate) const LAB_CA_KEY_FILE: &str = "ca.key";
/// Login shell on NixOS, which has no `/bin/bash`.
pub(crate) const NODE_NIXOS_SHELL: &str = "/run/current-system/sw/bin/bash";
/// RAM-backed server directory holding container secret files, under the daemon's
/// systemd `RuntimeDirectory` (`/run/sherpa`, mode 0700).
pub(crate) const SHERPA_SECRETS_DIR: &str = "/run/sherpa/secrets";
pub(crate) const READINESS_SLEEP: u64 = 10;
/// Keycloak HTTPS port.
pub(crate) const KEYCLOAK_HTTPS_PORT: u16 = 8443;
/// Realm Sherpa makes on Keycloak nodes; Kubernetes clusters accept its users.
pub(crate) const KEYCLOAK_REALM: &str = "sherpa";
/// Vault API (HTTPS) and cluster ports.
pub(crate) const VAULT_API_PORT: u16 = 8200;
/// The key that seals the database's secret values: 32 bytes, mode 0600, beside the JWT secret
/// in the `sherpa`-only (0700) secrets directory. It is never in the database, its backups or
/// the database directory, and is backed up separately from them.
pub(crate) const SHERPA_DB_KEY_PATH: &str = "/opt/sherpa/.secret/db.key";
pub(crate) const JWT_TOKEN_EXPIRY_SECONDS: i64 = 604_800; // 7 days
/// How often sherpad deletes expired login sessions, in seconds. It also sweeps once at startup.
pub(crate) const SESSION_SWEEP_INTERVAL_SECS: u64 = 3600;
