use std::net::Ipv4Addr;

use ipnet::{Ipv4Net, Ipv6Net};

use shared::data::{Dns, NameServer, NetworkV4, NetworkV6, SshKeyAlgorithms, SshPublicKey, User};

pub fn test_user() -> User {
    User {
        username: "sherpa".to_string(),
        password: Some("Everest1953!".to_string()),
        ssh_public_key: SshPublicKey {
            algorithm: SshKeyAlgorithms::SshRsa,
            key: "AAAAB3NzaC1yc2EAAAADAQABAAABAQ".to_string(),
            comment: Some("test@sherpa".to_string()),
        },
        sudo: true,
    }
}

pub fn test_dns() -> Dns {
    Dns {
        domain: "lab.sherpa.local".to_string(),
        name_servers: vec![NameServer {
            name: "ns1".to_string(),
            ipv4_address: Ipv4Addr::new(172, 20, 0, 1),
            ipv6_address: None,
        }],
    }
}

pub fn test_network_v4() -> NetworkV4 {
    NetworkV4 {
        prefix: "172.20.0.0/24".parse::<Ipv4Net>().expect("valid prefix"),
        first: Ipv4Addr::new(172, 20, 0, 1),
        last: Ipv4Addr::new(172, 20, 0, 254),
        boot_server: Ipv4Addr::new(172, 20, 0, 1),
        network: Ipv4Addr::new(172, 20, 0, 0),
        subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
        hostmask: Ipv4Addr::new(0, 0, 0, 255),
        prefix_length: 24,
    }
}

pub fn test_network_v6() -> NetworkV6 {
    NetworkV6 {
        prefix: "fd00::/64".parse::<Ipv6Net>().expect("valid prefix"),
        first: "fd00::1".parse().expect("valid addr"),
        last: "fd00::ffff".parse().expect("valid addr"),
        boot_server: "fd00::1".parse().expect("valid addr"),
        network: "fd00::".parse().expect("valid addr"),
        prefix_length: 64,
    }
}
