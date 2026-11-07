//! Each device template with the lab owner's account added next to `sherpa`.
use std::net::Ipv4Addr;

use askama::Template;

use template::{
    AristaVeosZtpTemplate, ArubaAoscxTemplate, CiscoAsavZtpTemplate, CiscoIosXeZtpTemplate,
    CiscoIosvZtpTemplate, CiscoIosxrZtpTemplate, CiscoNxosZtpTemplate, CumulusLinuxZtpTemplate,
    FrrStartupTemplate, JunipervJunosZtpTemplate, MikrotikRouterosZtpTemplate,
    PaloAltoPanosBootstrapTemplate, SonicLinuxUserTemplate, WindowsSshdConfigTemplate,
    build_srlinux_config,
};

use crate::helpers;

fn mgmt_address() -> Ipv4Addr {
    Ipv4Addr::new(172, 20, 0, 10)
}

#[test]
fn arista_owner_has_a_key_and_a_secondary_key() {
    let output = AristaVeosZtpTemplate {
        hostname: "veos01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(
        "username sherpa ssh-key ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ\n\
         username alice privilege 15 secret Rand0mSecret\n\
         username alice ssh-key ssh-rsa OWNERKEY1\n\
         username alice ssh-key secondary ssh-ed25519 OWNERKEY2\n!\n"
    ));
}

#[test]
fn aruba_owner_is_an_administrator_with_every_key() {
    let output = ArubaAoscxTemplate {
        hostname: "aoscx01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(
        "user alice group administrators password plaintext Rand0mSecret\n\
         user alice authorized-key ssh-rsa OWNERKEY1\n\
         user alice authorized-key ssh-ed25519 OWNERKEY2\n!\n"
    ));
}

#[test]
fn asa_owner_has_no_password_and_hashed_keys() {
    let output = CiscoAsavZtpTemplate {
        hostname: "asa01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(
        output.contains("username sherpa password Everest1953!\nusername alice privilege 15\n")
    );
    assert!(!output.contains("username alice password"));
    assert!(output.contains(
        "username alice attributes\n  \
         ssh authentication publickey OWNERKEY1 hashed\n  \
         ssh authentication publickey OWNERKEY2 hashed\n  \
         service-type admin\n!\n"
    ));
}

#[test]
fn ios_owner_has_a_random_secret_and_key_hashes() {
    let output = CiscoIosvZtpTemplate {
        hostname: "iosv01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
        mgmt_interface: "GigabitEthernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(
        "username sherpa privilege 15 secret Everest1953!\n\
         username alice privilege 15 secret Rand0mSecret\n!\n\
         ip ssh pubkey-chain\n  \
         username sherpa\n   \
         key-hash ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ\n  \
         username alice\n   \
         key-hash ssh-rsa OWNERKEY1\n   \
         key-hash ssh-ed25519 OWNERKEY2\n!\n"
    ));
}

#[test]
fn iosxe_owner_has_a_random_secret_and_key_hashes() {
    let output = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
        mgmt_interface: "GigabitEthernet1".to_string(),
        dns: helpers::test_dns(),
        license_boot_command: None,
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains("username alice privilege 15 secret Rand0mSecret\n"));
    assert!(output.contains(
        "  username alice\n   key-hash ssh-rsa OWNERKEY1\n   key-hash ssh-ed25519 OWNERKEY2\n!\n"
    ));
}

#[test]
fn iosxr_owner_keys_are_numbered() {
    let output = CiscoIosxrZtpTemplate {
        hostname: "xr01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(
        output.contains(
            "username alice\n group root-lr\n group netadmin\n secret 0 Rand0mSecret\n!\n"
        )
    );
    assert!(output.contains(
        "ssh server username alice\n \
         keystring ssh-rsa OWNERKEY1\n \
         keystring ssh-ed25519 second OWNERKEY2\n!\n"
    ));
}

#[test]
fn nxos_owner_is_a_network_admin() {
    let output = CiscoNxosZtpTemplate {
        hostname: "nxos01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(
        "username alice password 0 Rand0mSecret role network-admin\n\
         username alice sshkey ssh-rsa OWNERKEY1\n"
    ));
}

#[test]
fn cumulus_owner_has_a_named_key_each_and_the_admin_role() {
    let output = CumulusLinuxZtpTemplate {
        hostname: "cumulus01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(
        "nv set system aaa user alice\n\
         nv set system aaa user alice password 'Rand0mSecret'\n\
         nv set system aaa user alice ssh authorized-key alice-ssh-key-1 key OWNERKEY1\n\
         nv set system aaa user alice ssh authorized-key alice-ssh-key-1 type ssh-rsa\n\
         nv set system aaa user alice ssh authorized-key alice-ssh-key-2 key OWNERKEY2\n\
         nv set system aaa user alice ssh authorized-key alice-ssh-key-2 type ssh-ed25519\n\
         nv set system aaa user alice role system-admin\n"
    ));
}

#[test]
fn frr_owner_logs_in_by_key_only() {
    let output = FrrStartupTemplate {
        hostname: "frr01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
    }
    .render()
    .unwrap();
    assert!(output.contains("adduser -D -s /bin/sh alice\n"));
    assert!(output.contains("echo 'alice:*' | chpasswd -e\n"));
    assert!(output.contains("echo 'alice ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/alice\n"));
    assert!(output.contains(
        "cat > /home/alice/.ssh/authorized_keys <<'SHERPA_KEYS'\n\
         ssh-rsa OWNERKEY1\nssh-ed25519 OWNERKEY2\nSHERPA_KEYS\n"
    ));
    assert!(!output.contains("Rand0mSecret"));
}

#[test]
fn junos_owner_is_a_super_user_with_every_key() {
    let mut owner = helpers::test_owner();
    owner.ssh_keys[0].value = "ssh-rsa OWNERKEY1".to_string();
    let output = JunipervJunosZtpTemplate {
        hostname: "vsrx01".to_string(),
        user: helpers::test_user(),
        owner: Some(owner),
        mgmt_interface: "fxp0".to_string(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(
        r#"        user alice {
            class super-user;
            authentication {
                ssh-rsa "ssh-rsa OWNERKEY1"; ## SECRET-DATA
                ssh-ed25519 "OWNERKEY2"; ## SECRET-DATA
            }
        }
"#
    ));
    assert!(!output.contains("Rand0mSecret"));
}

#[test]
fn mikrotik_owner_imports_each_key_file() {
    let output = MikrotikRouterosZtpTemplate {
        hostname: "mikrotik01".to_string(),
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
        owner_key_files: vec![
            MikrotikRouterosZtpTemplate::owner_key_path("alice", 1),
            MikrotikRouterosZtpTemplate::owner_key_path("alice", 2),
        ],
        mgmt_interface: "ether1".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(
        ":do { /user add name=alice password=Rand0mSecret group=full } on-error={}\n\
         /user ssh-keys import public-key-file=sata1/alice-1.pub user=alice\n\
         /user ssh-keys import public-key-file=sata1/alice-2.pub user=alice\n"
    ));
}

#[test]
fn sonic_owner_has_a_locked_password_and_sudo_without_one() {
    let output = SonicLinuxUserTemplate {
        user: helpers::test_user(),
        owner: Some(helpers::test_owner()),
    }
    .render()
    .unwrap();
    assert!(output.contains("/usr/sbin/useradd -m -s /bin/bash -G sudo,admin,docker alice\n"));
    assert!(
        output.contains("<<'SHERPA_KEYS'\nssh-rsa OWNERKEY1\nssh-ed25519 OWNERKEY2\nSHERPA_KEYS\n")
    );
    assert!(output.contains("'alice ALL=(ALL) NOPASSWD: ALL' >/etc/sudoers.d/alice\n"));
    // No password: the account stays locked.
    assert!(!output.contains(r#""alice:"#));
}

#[test]
fn srlinux_owner_is_an_admin_user_with_every_key() {
    let owner = helpers::test_owner();
    let json = build_srlinux_config(
        "srl01",
        &helpers::test_user(),
        Some(&owner),
        &helpers::test_dns(),
        &helpers::test_network_v4(),
        Some(mgmt_address()),
        None,
    )
    .unwrap();
    let config: serde_json::Value = serde_json::from_str(&json).unwrap();
    let users = &config["srl_nokia-system:system"]["srl_nokia-aaa:aaa"]["authentication"]["user"];
    assert_eq!(users[0]["username"], "sherpa");
    assert_eq!(users[1]["username"], "alice");
    assert_eq!(users[1]["password"], "Rand0mSecret");
    assert_eq!(
        users[1]["ssh-key"],
        serde_json::json!(["ssh-rsa OWNERKEY1", "ssh-ed25519 OWNERKEY2"])
    );
    assert_eq!(users[1]["role"], serde_json::json!(["srl-admin"]));
}

#[test]
fn panos_bootstrap_escapes_xml() {
    let mut owner = helpers::test_owner();
    owner.ssh_keys.truncate(1);
    owner.ssh_keys[0].value = "KEY&<1".to_string();
    let output = PaloAltoPanosBootstrapTemplate {
        hostname: "pan01".to_string(),
        user: helpers::test_user(),
        owner: Some(owner),
        password_hash: "$5$sherpa$hash".to_string(),
        ssh_public_key_b64: "SHERPAKEY".to_string(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        mgmt_netmask: Ipv4Addr::new(255, 255, 255, 0),
        mgmt_gateway: Ipv4Addr::new(172, 20, 0, 1),
        dns_primary: Ipv4Addr::new(172, 20, 0, 1),
    }
    .render()
    .unwrap();
    assert!(
        output.contains("<public-key>KEY&#38;&#60;1</public-key>"),
        "{output}"
    );
    assert!(output.contains("<phash>$5$sherpa$hash</phash>"), "{output}");
}

#[test]
fn panos_owner_is_a_superuser_with_its_own_hash_and_key() {
    let mut owner = helpers::test_owner();
    owner.secret = "$5$owner$hash".to_string();
    owner.ssh_keys.truncate(1);
    let output = PaloAltoPanosBootstrapTemplate {
        hostname: "pan01".to_string(),
        user: helpers::test_user(),
        owner: Some(owner),
        password_hash: "$5$sherpa$hash".to_string(),
        ssh_public_key_b64: "SHERPAKEY".to_string(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        mgmt_netmask: Ipv4Addr::new(255, 255, 255, 0),
        mgmt_gateway: Ipv4Addr::new(172, 20, 0, 1),
        dns_primary: Ipv4Addr::new(172, 20, 0, 1),
    }
    .render()
    .unwrap();
    assert!(output.contains(
        r#"      <entry name="alice">
        <phash>$5$owner$hash</phash>
        <permissions>
          <role-based>
            <superuser>yes</superuser>
          </role-based>
        </permissions>
        <public-key>OWNERKEY1</public-key>
      </entry>
    </users>"#
    ));
}

#[test]
fn windows_sshd_config_reads_each_accounts_own_keys_file() {
    let output = WindowsSshdConfigTemplate {
        authorized_keys_dir: "__PROGRAMDATA__/ssh/authorized_keys".to_string(),
    }
    .render()
    .unwrap();
    assert!(output.contains("AuthorizedKeysFile __PROGRAMDATA__/ssh/authorized_keys/%u\n"));
    assert!(output.contains("Subsystem sftp sftp-server.exe"));
    assert!(!output.contains("Match Group administrators"));
}
