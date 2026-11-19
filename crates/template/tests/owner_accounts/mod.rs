//! Each device template with the lab owner as the one login: the owner's keys in each device's
//! own format, and no other account but the device's fixed vendor ones.
use std::net::Ipv4Addr;

use askama::Template;

use template::{
    PaloAltoPanosBootstrapTemplate, WindowsOwnerOnlyLoginTemplate, WindowsSshdConfigTemplate,
};

use crate::helpers;

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
}
