use std::net::Ipv4Addr;

use ipnet::{Ipv4Net, Ipv6Net};

use shared::data::{
    BiosTypes, CpuArchitecture, CpuModels, DiskBuses, Dns, InterfaceType, MachineType,
    MgmtInterfaces, NameServer, NetworkV4, NetworkV6, NodeConfig, NodeKind, NodeModel, OsVariant,
    SshKeyAlgorithms, SshPublicKey, User, ZtpMethod,
};

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

pub fn test_node_config(model: NodeModel) -> NodeConfig {
    NodeConfig {
        id: None,
        model,
        version: "1.0.0".to_string(),
        repo: Some("test-repo".to_string()),
        os_variant: OsVariant::Linux,
        kind: NodeKind::VirtualMachine,
        bios: BiosTypes::SeaBios,
        cpu_count: 1,
        cpu_architecture: CpuArchitecture::X86_64,
        cpu_model: CpuModels::HostModel,
        machine_type: MachineType::Q35,
        vmx_enabled: false,
        memory: 1024,
        hdd_bus: DiskBuses::Virtio,
        cdrom: None,
        cdrom_bus: DiskBuses::Sata,
        ztp_enable: false,
        ztp_method: ZtpMethod::None,
        ztp_username: None,
        ztp_password: None,
        ztp_password_auth: false,
        data_interface_count: 4,
        interface_prefix: "eth".to_string(),
        interface_type: InterfaceType::Virtio,
        interface_mtu: 1500,
        first_interface_index: 0,
        dedicated_management_interface: false,
        management_interface: MgmtInterfaces::Eth0,
        reserved_interface_count: 0,
        default: false,
    }
}
