use std::net::Ipv4Addr;

use askama::Template;

use template::MikrotikRouterosZtpTemplate;

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_STATIC_IPV4: &str = "\
/system identity set name=mikrotik01
/ip dns set servers=172.20.0.1
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
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    }
}

#[test]
fn test_static_ipv4() {
    let output = routeros("Rand0mPass-1", &[])
        .render()
        .expect("template renders");
    assert_eq!(
        output,
        EXPECTED_STATIC_IPV4.replace("PASSWORD", "Rand0mPass-1")
    );
}

#[test]
fn test_only_the_first_user_and_the_owner_have_accounts() {
    let output = routeros("Rand0mPass-1", &["alice-1.pub", "alice-2.pub"])
        .render()
        .expect("template renders");
    let users: Vec<&str> = output
        .lines()
        .filter(|line| line.contains("/user "))
        .collect();
    assert_eq!(
        users,
        [
            "/user set 0 password=Rand0mPass-1",
            ":do { /user add name=alice password=Rand0mPass-1 group=full } on-error={}",
            "/user ssh-keys import public-key-file=alice-1.pub user=alice",
            "/user ssh-keys import public-key-file=alice-2.pub user=alice",
        ]
    );
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn test_password_is_intact_for_user_zero_and_owner() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let output = routeros(password, &[]).render().expect("template renders");
        assert!(output.contains(&format!("\n/user set 0 password={password}\n")));
        assert!(output.contains(&format!(
            "\n:do {{ /user add name=alice password={password} group=full }} on-error={{}}\n"
        )));
        assert_eq!(output.matches(password).count(), 2);
    }
}
