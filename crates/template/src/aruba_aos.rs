use std::net::{Ipv4Addr, Ipv6Addr};

use askama::Template;

use shared::data::{Dns, NetworkV4, NetworkV6, OwnerSshKey};

/// The AOS-CX `ciphertext` form of a random password nobody keeps, which locks the accounts it
/// is set on: the factory `admin`, whose factory password is empty and which the switch refuses
/// to delete, and the owner, who logs in by key only. An account with no password at all takes
/// an empty one on the console and on the HTTPS REST API, whatever `ssh password-authentication`
/// says.
///
/// Made once on an AOS-CX 10.16.1010 virtual switch: a 32-character password from
/// `openssl rand` was set with `user admin password plaintext` and never shown or stored, and
/// this is the ciphertext the running config then held. The switch encrypts it under the
/// default export password, a key every AOS-CX switch shares, so it loads on any 10.16 switch
/// that keeps that default. Decrypted, it gives at most a salted hash of that random password,
/// and nothing about the device password, the owner or the switch it was made on.
const LOCKED_PASSWORD: &str = "AQBapWi7gAb+1N611aRfybWmroBDwZuBCFmatCf0xTBVaKfvYgAAAG28Q0mssiKLsEYc6oFjpoOK7se339jlbWQZ6TgkNwA+Fds+jXCbsS/YVarj2xHh4ddtZwgVPlwNJnOn0qS3wdj67Tiu4rBBELar7IBH8ADbnYa4fUYPGL79+0g2OlJGNQ5p";

/// An AOS-CX switch's ZTP config. The owner, `username`, logs in over SSH with `ssh_keys`
/// only: SSH password login is off, and the owner and `admin` are locked. The device password
/// is not in it.
#[derive(Template)]
#[template(path = "aruba/aruba_aoscx.jinja", ext = "txt")]
pub struct ArubaAoscxTemplate {
    pub hostname: String,
    pub username: String,
    pub ssh_keys: Vec<OwnerSshKey>,
    pub dns: Dns,
    pub mgmt_ipv4: NetworkV4,
    pub mgmt_ipv4_address: Ipv4Addr,
    pub mgmt_ipv6_address: Option<Ipv6Addr>,
    pub mgmt_ipv6: Option<NetworkV6>,
}
