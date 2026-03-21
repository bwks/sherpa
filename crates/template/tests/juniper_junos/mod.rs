use std::net::Ipv4Addr;

use askama::Template;

use template::JunipervJunosZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = r#"system {
    login {
        user sherpa {
            class super-user;
            authentication {
                plain-text-password-value "Everest1953!";
                ssh-rsa "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ"; ## SECRET-DATA
            }
        }
    }
    root-authentication {
        plain-text-password-value "Everest1953!";
        ssh-rsa "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ"; ## SECRET-DATA
    }
    host-name vsrx01;
    services {
        ssh {
            root-login allow;
        }
        netconf {
            ssh;
        }
    }
}
routing-options {
    static {
        route 0.0.0.0/0 next-hop 172.20.0.1;
    }
}
interfaces {
    fxp0 {
        unit 0 {
            family inet {
                address 172.20.0.10/24;
            }
        }
    }
}
protocols {
    lldp {
        interface all;
    }
}"#;

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_static_ipv4() {
    let t = JunipervJunosZtpTemplate {
        hostname: "vsrx01".to_string(),
        user: helpers::test_user(),
        mgmt_interface: "fxp0".to_string(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4);
}
