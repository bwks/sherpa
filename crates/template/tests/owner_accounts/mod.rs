//! Each device template with the lab owner as the one login: the owner's keys in each device's
//! own format, and no other account but the device's fixed vendor ones.
use std::net::Ipv4Addr;

use askama::Template;

use template::{
    AristaVeosZtpTemplate, ArubaAoscxTemplate, CiscoAsavZtpTemplate, CiscoIosXeZtpTemplate,
    CiscoIosvZtpTemplate, CiscoIosxrZtpTemplate, CiscoNxosZtpTemplate, CumulusLinuxZtpTemplate,
    FrrStartupTemplate, JunipervJunosZtpTemplate, MikrotikRouterosZtpTemplate,
    PaloAltoPanosBootstrapTemplate, SonicLinuxUserTemplate, WindowsOwnerOnlyLoginTemplate,
    WindowsSshdConfigTemplate, build_srlinux_config,
};

use crate::helpers;

const PASSWORD: &str = "Rand0mPass-1";

fn mgmt_address() -> Ipv4Addr {
    Ipv4Addr::new(172, 20, 0, 10)
}

#[test]
fn arista_owner_has_a_key_and_a_secondary_key() {
    let hash = helpers::sha512_hash(PASSWORD);
    let output = AristaVeosZtpTemplate {
        hostname: "veos01".to_string(),
        owner: helpers::test_owner(&hash),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(&format!(
        "aaa authorization exec default local\n!\n\
         username alice privilege 15 secret sha512 {hash}\n\
         username alice ssh-key ssh-rsa OWNERKEY1\n\
         username alice ssh-key secondary ssh-ed25519 OWNERKEY2\n!\n"
    )));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn aruba_owner_is_an_administrator_with_every_key() {
    let output = ArubaAoscxTemplate {
        hostname: "aoscx01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(&format!(
        "user admin group administrators password plaintext {PASSWORD}\n\
         user alice group administrators password plaintext {PASSWORD}\n\
         user alice authorized-key ssh-rsa OWNERKEY1\n\
         user alice authorized-key ssh-ed25519 OWNERKEY2\n!\n"
    )));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn asa_owner_has_a_password_and_hashed_keys() {
    let output = CiscoAsavZtpTemplate {
        hostname: "asa01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(&format!(
        "username enable_1 privilege 15\n\
         enable password {PASSWORD}\n\
         username alice password {PASSWORD} privilege 15\n"
    )));
    assert!(output.contains(
        "username alice attributes\n  \
         ssh authentication publickey OWNERKEY1 hashed\n  \
         ssh authentication publickey OWNERKEY2 hashed\n  \
         service-type admin\n!\n"
    ));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn ios_owner_has_a_secret_and_key_hashes() {
    let output = CiscoIosvZtpTemplate {
        hostname: "iosv01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        mgmt_interface: "GigabitEthernet0/0".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(&format!(
        "username alice privilege 15 secret {PASSWORD}\n!\n\
         ip ssh pubkey-chain\n  \
         username alice\n   \
         key-hash ssh-rsa OWNERKEY1\n   \
         key-hash ssh-ed25519 OWNERKEY2\n!\n"
    )));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn iosxe_owner_has_a_secret_and_key_hashes() {
    let output = CiscoIosXeZtpTemplate {
        hostname: "csr01".to_string(),
        owner: helpers::test_owner(PASSWORD),
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
    assert!(output.contains(&format!("username alice privilege 15 secret {PASSWORD}\n")));
    assert!(output.contains(
        "  username alice\n   key-hash ssh-rsa OWNERKEY1\n   key-hash ssh-ed25519 OWNERKEY2\n!\n"
    ));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn iosxr_owner_keys_are_numbered() {
    let output = CiscoIosxrZtpTemplate {
        hostname: "xr01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(&format!(
        "username alice\n group root-lr\n group netadmin\n secret 0 {PASSWORD}\n!\n"
    )));
    assert!(output.contains(
        "ssh server username alice\n \
         keystring ssh-rsa OWNERKEY1\n \
         keystring ssh-ed25519 second OWNERKEY2\n!\n"
    ));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn nxos_owner_is_a_network_admin_with_every_key() {
    let output = CiscoNxosZtpTemplate {
        hostname: "nxos01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(&format!(
        "username alice password 0 {PASSWORD} role network-admin\n\
         username alice sshkey ssh-rsa OWNERKEY1\n\
         username alice sshkey ssh-ed25519 OWNERKEY2\n"
    )));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn cumulus_owner_has_a_named_key_each_and_the_admin_role() {
    let output = CumulusLinuxZtpTemplate {
        hostname: "cumulus01".to_string(),
        owner: helpers::test_owner(PASSWORD),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(&format!(
        "nv set system aaa user alice\n\
         nv set system aaa user alice password '{PASSWORD}'\n\
         nv set system aaa user alice ssh authorized-key alice-ssh-key-1 key OWNERKEY1\n\
         nv set system aaa user alice ssh authorized-key alice-ssh-key-1 type ssh-rsa\n\
         nv set system aaa user alice ssh authorized-key alice-ssh-key-2 key OWNERKEY2\n\
         nv set system aaa user alice ssh authorized-key alice-ssh-key-2 type ssh-ed25519\n\
         nv set system aaa user alice role system-admin\n"
    )));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn frr_owner_has_its_hash_every_key_and_sudo() {
    let hash = helpers::sha512_hash(PASSWORD);
    let output = FrrStartupTemplate {
        hostname: "frr01".to_string(),
        owner: helpers::test_owner(&hash),
    }
    .render()
    .unwrap();
    assert!(output.contains("adduser -D -s /bin/sh alice\n"));
    assert!(output.contains(&format!("echo 'alice:{hash}' | chpasswd -e\n")));
    assert!(output.contains("echo 'alice ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/alice\n"));
    assert!(output.contains(
        "cat > /home/alice/.ssh/authorized_keys <<'SHERPA_KEYS'\n\
         ssh-rsa OWNERKEY1\nssh-ed25519 OWNERKEY2\nSHERPA_KEYS\n"
    ));
    assert!(!output.contains(PASSWORD));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn junos_owner_is_a_super_user_with_every_key() {
    let hash = helpers::sha512_hash(PASSWORD);
    let mut owner = helpers::test_owner(&hash);
    owner.ssh_keys[0].value = "ssh-rsa OWNERKEY1".to_string();
    let output = JunipervJunosZtpTemplate {
        hostname: "vsrx01".to_string(),
        owner,
        mgmt_interface: "fxp0".to_string(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(mgmt_address()),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
    .render()
    .unwrap();
    assert!(output.contains(&format!(
        r#"        user alice {{
            class super-user;
            authentication {{
                encrypted-password "{hash}"; ## SECRET-DATA
                ssh-rsa "ssh-rsa OWNERKEY1"; ## SECRET-DATA
                ssh-ed25519 "OWNERKEY2"; ## SECRET-DATA
            }}
        }}
"#
    )));
    assert!(!output.contains(PASSWORD));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn mikrotik_owner_imports_each_key_file() {
    let output = MikrotikRouterosZtpTemplate {
        hostname: "mikrotik01".to_string(),
        owner: helpers::test_owner(PASSWORD),
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
    assert!(output.contains(&format!(
        ":do {{ /user add name=alice password={PASSWORD} group=full }} on-error={{}}\n\
         :if ([/file find name=sata1/alice-1.pub] != \"\") do={{ /user ssh-keys import public-key-file=sata1/alice-1.pub user=alice }}\n\
         :if ([/file find name=sata1/alice-2.pub] != \"\") do={{ /user ssh-keys import public-key-file=sata1/alice-2.pub user=alice }}\n"
    )));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn sonic_owner_has_its_hash_every_key_and_sudo() {
    let hash = helpers::sha512_hash(PASSWORD);
    let output = SonicLinuxUserTemplate {
        owner: helpers::test_owner(&hash),
    }
    .render()
    .unwrap();
    assert!(output.contains("/usr/sbin/useradd -m -s /bin/bash -G sudo,admin,docker alice\n"));
    assert!(output.contains(&format!(
        "/usr/bin/echo 'alice:{hash}' | /usr/sbin/chpasswd -e\n"
    )));
    assert!(
        output.contains("<<'SHERPA_KEYS'\nssh-rsa OWNERKEY1\nssh-ed25519 OWNERKEY2\nSHERPA_KEYS\n")
    );
    assert!(output.contains("'alice ALL=(ALL) NOPASSWD: ALL' >/etc/sudoers.d/alice\n"));
    assert!(!output.contains(PASSWORD));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn srlinux_owner_is_an_admin_user_with_every_key() {
    let json = build_srlinux_config(
        "srl01",
        &helpers::test_owner(PASSWORD),
        &helpers::test_dns(),
        &helpers::test_network_v4(),
        Some(mgmt_address()),
        None,
    )
    .unwrap();
    let config: serde_json::Value = serde_json::from_str(&json).unwrap();
    let users = &config["srl_nokia-system:system"]["srl_nokia-aaa:aaa"]["authentication"]["user"];
    assert_eq!(users.as_array().map(Vec::len), Some(1));
    assert_eq!(users[0]["username"], "alice");
    assert_eq!(users[0]["password"], PASSWORD);
    assert_eq!(
        users[0]["ssh-key"],
        serde_json::json!(["ssh-rsa OWNERKEY1", "ssh-ed25519 OWNERKEY2"])
    );
    assert_eq!(users[0]["role"], serde_json::json!(["srl-admin"]));
    helpers::assert_no_sherpa_login(&json);
}

#[test]
fn panos_bootstrap_escapes_xml() {
    let mut owner = helpers::test_owner("$5$salt$hash");
    owner.ssh_keys.truncate(1);
    owner.ssh_keys[0].value = "KEY&<1".to_string();
    let output = PaloAltoPanosBootstrapTemplate {
        hostname: "pan01".to_string(),
        owner,
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
}

#[test]
fn panos_owner_is_a_superuser_with_its_own_hash_and_key() {
    let hash = helpers::sha256_hash(PASSWORD);
    let mut owner = helpers::test_owner(&hash);
    owner.ssh_keys.truncate(1);
    let output = PaloAltoPanosBootstrapTemplate {
        hostname: "pan01".to_string(),
        owner,
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
        mgmt_netmask: Ipv4Addr::new(255, 255, 255, 0),
        mgmt_gateway: Ipv4Addr::new(172, 20, 0, 1),
        dns_primary: Ipv4Addr::new(172, 20, 0, 1),
    }
    .render()
    .unwrap();
    assert!(output.contains(&format!(
        r#"      <entry name="alice">
        <phash>{hash}</phash>
        <permissions>
          <role-based>
            <superuser>yes</superuser>
          </role-based>
        </permissions>
        <public-key>OWNERKEY1</public-key>
      </entry>
    </users>"#
    )));
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn windows_sshd_config_reads_each_accounts_own_keys_file() {
    let output = WindowsSshdConfigTemplate {
        authorized_keys_dir: "__PROGRAMDATA__/ssh/authorized_keys".to_string(),
        ready_marker: "sherpa-ready".to_string(),
    }
    .render()
    .unwrap();
    assert!(output.contains("AuthorizedKeysFile __PROGRAMDATA__/ssh/authorized_keys/%u\n"));
    assert!(
        output.contains("\nVersionAddendum sherpa-ready"),
        "{output}"
    );
    assert!(output.contains("Subsystem sftp sftp-server.exe"));
    assert!(!output.contains("Match Group administrators"));
}

#[test]
fn windows_leaves_the_owner_as_the_only_enabled_account() {
    let output = WindowsOwnerOnlyLoginTemplate {
        owner: "alice".to_string(),
    }
    .render()
    .unwrap();
    let commands: Vec<&str> = output
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    assert_eq!(
        commands,
        [
            "$ErrorActionPreference = 'Stop'",
            "$owner = Get-LocalUser -Name 'alice'",
            "if (-not $owner.Enabled) {",
            "    throw \"The lab owner's account 'alice' is disabled, so no other account was disabled\"",
            "}",
            "Get-LocalUser |",
            "    Where-Object { $_.Enabled -and $_.SID -ne $owner.SID } |",
            "    Disable-LocalUser",
        ]
    );
    // Administrator and Admin are among the accounts disabled, never enabled or given a
    // password.
    for account in ["Administrator", "Admin"] {
        assert!(!output.contains(&format!("-Name '{account}'")), "{output}");
    }
    assert!(!output.contains("Enable-LocalUser"));
    assert!(!output.contains("Set-LocalUser"));
    helpers::assert_no_sherpa_login(&output);
}
