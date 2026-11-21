use std::net::Ipv4Addr;

use askama::Template;

use template::{SshConfigTemplate, SshHostConfig};

const OLDER_ALGORITHMS: [&str; 3] = [
    "    PubkeyAcceptedAlgorithms +ssh-rsa",
    "    HostkeyAlgorithms +ssh-rsa",
    "    KexAlgorithms +diffie-hellman-group-exchange-sha1,diffie-hellman-group14-sha1",
];

fn host(name: &str, last_octet: u8, known_hosts_file: Option<&str>) -> SshHostConfig {
    SshHostConfig {
        aliases: format!("172.20.0.{last_octet} {name}.abcd1234 {name}.abcd1234.sherpa.lab.local"),
        username: "alice".to_owned(),
        known_hosts_file: known_hosts_file.map(str::to_owned),
        hostname: Ipv4Addr::new(172, 20, 0, last_octet),
        port: 22,
    }
}

/// The lines of each host of `output`, in order.
fn hosts(output: &str) -> Vec<Vec<&str>> {
    output
        .split("\nHost ")
        .skip(1)
        .map(|host| host.lines().collect())
        .collect()
}

#[test]
fn ssh_config_hosts_006_every_host_accepts_the_older_algorithms() {
    let t = SshConfigTemplate {
        hosts: vec![
            host("router1", 10, None),
            host("switch1", 11, Some("sherpa_known_hosts")),
        ],
    };

    let output = t.render().expect("template renders");

    let hosts = hosts(&output);
    assert_eq!(hosts.len(), 2, "{output}");
    for host in hosts {
        for algorithm in OLDER_ALGORITHMS {
            assert!(host.contains(&algorithm), "{algorithm} in {host:?}");
        }
    }
}

#[test]
fn node_host_keys_007_a_host_with_a_recorded_key_is_checked_strictly() {
    let t = SshConfigTemplate {
        hosts: vec![
            host("router1", 10, Some("sherpa_known_hosts")),
            host("switch1", 11, None),
        ],
    };

    let output = t.render().expect("template renders");

    let hosts = hosts(&output);
    assert_eq!(hosts.len(), 2, "{output}");
    assert!(
        hosts[0].contains(&"    StrictHostKeyChecking yes"),
        "{output}"
    );
    assert!(
        hosts[0].contains(&"    UserKnownHostsFile sherpa_known_hosts"),
        "{output}"
    );
    assert!(
        hosts[1].contains(&"    StrictHostKeyChecking no"),
        "{output}"
    );
    assert!(
        hosts[1].contains(&"    UserKnownHostsFile /dev/null"),
        "{output}"
    );
}
