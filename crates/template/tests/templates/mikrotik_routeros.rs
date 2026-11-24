use std::net::Ipv4Addr;

use askama::Template;

use template::MikrotikRouterosZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "\
/system identity set name=mikrotik01
/ip dns set servers=172.20.0.2
/user set 0 password=PASSWORD
:do { /user add name=alice password=PASSWORD group=full } on-error={}
/ip address add address=172.20.0.10/24 interface=ether1
/ip route add dst-address=0.0.0.0/0 gateway=172.20.0.1
/ip service set ssh disabled=no
/ip service set telnet disabled=yes
/ip service set www disabled=yes
/ip service set api disabled=yes
/ip ssh set strong-crypto=yes";

// ============================================================================
// Tests
// ============================================================================

fn routeros(password: &str, key_files: &[&str]) -> MikrotikRouterosZtpTemplate {
    MikrotikRouterosZtpTemplate {
        hostname: "mikrotik01".to_string(),
        owner: helpers::test_owner(password),
        owner_key_files: key_files.iter().map(|file| (*file).to_string()).collect(),
        mgmt_interface: "ether1".to_string(),
        dns: helpers::test_dns(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Ipv4Addr::new(172, 20, 0, 10),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn generated_config_019_mikrotik_routeros() {
    let output = routeros("Rand0mPass-1", &[])
        .render()
        .expect("template renders");
    assert_eq!(
        output,
        EXPECTED_STATIC_IPV4.replace("PASSWORD", "Rand0mPass-1")
    );
}

/// The image's scheduler imports the config every minute until an import gets to the end.
/// `RouterOS` deletes a key file once it imports it, so a retry, such as after the first run
/// found no `ether1` yet, must skip a key it imported before rather than stop on it.
#[test]
fn device_accounts_010_routeros_first_user_and_owner_are_the_only_accounts() {
    let mut template = routeros("Rand0mPass-1", &[]);
    template.owner_key_files = vec![
        MikrotikRouterosZtpTemplate::owner_key_path("alice", 1),
        MikrotikRouterosZtpTemplate::owner_key_path("alice", 2),
    ];
    let output = template.render().expect("template renders");
    let users: Vec<&str> = output
        .lines()
        .filter(|line| line.contains("/user ") && !line.contains("ssh-keys"))
        .collect();
    assert_eq!(
        users,
        [
            "/user set 0 password=Rand0mPass-1",
            ":do { /user add name=alice password=Rand0mPass-1 group=full } on-error={}",
        ]
    );
}

#[test]
fn generated_config_025_routeros_imports_each_key_file_only_if_it_is_there() {
    let mut template = routeros("Rand0mPass-1", &[]);
    template.owner_key_files = vec![
        MikrotikRouterosZtpTemplate::owner_key_path("alice", 1),
        MikrotikRouterosZtpTemplate::owner_key_path("alice", 2),
    ];
    let output = template.render().expect("template renders");
    let imports: Vec<&str> = output
        .lines()
        .filter(|line| line.contains("ssh-keys"))
        .collect();
    assert_eq!(
        imports,
        [
            r#":if ([/file find name=sata1/alice-1.pub] != "") do={ /user ssh-keys import public-key-file=sata1/alice-1.pub user=alice }"#,
            r#":if ([/file find name=sata1/alice-2.pub] != "") do={ /user ssh-keys import public-key-file=sata1/alice-2.pub user=alice }"#,
        ]
    );
}

#[test]
fn device_accounts_011_routeros_first_user_and_owner_passwords() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let output = routeros(password, &[]).render().expect("template renders");
        assert!(output.contains(&format!("\n/user set 0 password={password}\n")));
        assert!(output.contains(&format!(
            "\n:do {{ /user add name=alice password={password} group=full }} on-error={{}}\n"
        )));
        assert_eq!(output.matches(password).count(), 2);
    }
}

#[test]
fn config_name_server_013_mikrotik_routeros() {
    let output = routeros("Rand0mPass-1", &[])
        .render()
        .expect("template renders");
    assert_eq!(
        helpers::name_servers(&output, "/ip dns set servers="),
        ["172.20.0.2"]
    );
}
