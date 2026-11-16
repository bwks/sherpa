use std::net::Ipv4Addr;

use askama::Template;

use template::JunipervJunosZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = r#"system {
    login {
        user alice {
            class super-user;
            authentication {
                encrypted-password "HASH"; ## SECRET-DATA
                ssh-rsa "OWNERKEY1"; ## SECRET-DATA
                ssh-ed25519 "OWNERKEY2"; ## SECRET-DATA
            }
        }
    }
    root-authentication {
        encrypted-password "HASH"; ## SECRET-DATA
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

fn junos(hash: &str) -> JunipervJunosZtpTemplate {
    JunipervJunosZtpTemplate {
        hostname: "vsrx01".to_string(),
        owner: helpers::test_owner(hash),
        mgmt_interface: "fxp0".to_string(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn test_static_ipv4() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = junos(&hash).render().expect("template renders");
    assert_eq!(output, EXPECTED_STATIC_IPV4.replace("HASH", &hash));
}

#[test]
fn test_only_the_owner_and_root_have_accounts() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = junos(&hash).render().expect("template renders");
    let users: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("user ") || line.starts_with("root-authentication"))
        .collect();
    assert_eq!(users, ["user alice {", "root-authentication {"]);
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn test_password_is_a_hash_that_verifies_for_owner_and_root() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let hash = helpers::sha512_hash(password);
        let output = junos(&hash).render().expect("template renders");
        let found = helpers::hashes_after(&output, "encrypted-password \"");
        assert_eq!(found.len(), 2);
        for found in &found {
            assert_eq!(found, &hash);
            helpers::assert_hash_verifies(found, "$6$", password);
        }
        assert!(!output.contains(password));
        assert!(!output.contains("plain-text-password-value"));
    }
}
