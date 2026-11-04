use std::net::{Ipv4Addr, Ipv6Addr};

use askama::Template;

use shared::data::{Dns, NetworkV4, NetworkV6, OwnerUser, User};
use shared::konst::MIKROTIK_CONFIG_DISK;

#[derive(Template)]
#[template(path = "mikrotik/mikrotik_routeros.jinja", ext = "txt")]
pub struct MikrotikRouterosZtpTemplate {
    pub hostname: String,
    pub user: User,
    pub owner: Option<OwnerUser>,
    /// The owner's public key files on the config disk, as RouterOS paths, one key per file.
    pub owner_key_files: Vec<String>,
    pub mgmt_interface: String,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Option<Ipv4Addr>,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
}

impl MikrotikRouterosZtpTemplate {
    /// The name of the owner's `index`th public key file on the config disk, counted from 1.
    pub fn owner_key_filename(username: &str, index: usize) -> String {
        format!("{username}-{index}.pub")
    }

    /// The file's path as RouterOS sees it on the config disk.
    pub fn owner_key_path(username: &str, index: usize) -> String {
        format!(
            "{MIKROTIK_CONFIG_DISK}/{}",
            Self::owner_key_filename(username, index)
        )
    }
}
