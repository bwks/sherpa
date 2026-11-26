use askama::Template;

use std::net::{Ipv4Addr, Ipv6Addr};

use template::{
    SonicLinuxContainerInitTemplate, SonicLinuxContainerManagement,
    SonicLinuxContainerManagementV6, SonicLinuxContainerSupervisordTemplate,
};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const INIT_SCRIPT: &str = "/mnt/sherpa-ztp/sherpa-init.sh";
const CONFIG_DB: &str = "/mnt/sherpa-ztp/config_db.json";

const EXPECTED_SUPERVISORD: &str = "[supervisord]
nodaemon=true
user=root
logfile=/dev/null
logfile_maxbytes=0
pidfile=/run/sherpa-supervisord.pid

[program:sherpa-init]
command=/bin/bash /mnt/sherpa-ztp/sherpa-init.sh
autorestart=false
startsecs=0
stdout_logfile=/dev/fd/1
stdout_logfile_maxbytes=0
redirect_stderr=true";

fn management(ipv6: Option<SonicLinuxContainerManagementV6>) -> SonicLinuxContainerManagement {
    SonicLinuxContainerManagement {
        ipv4_address: Ipv4Addr::new(172, 31, 0, 23),
        ipv4_prefix_length: 24,
        ipv4_gateway: Ipv4Addr::new(172, 31, 0, 1),
        ipv6,
    }
}

fn init(hash: Option<&str>, config_db: Option<&str>) -> String {
    SonicLinuxContainerInitTemplate {
        data_count: 2,
        owner: hash.map(helpers::test_owner),
        config_db: config_db.map(str::to_owned),
        mgmt: management(None),
    }
    .render()
    .expect("template renders")
}

/// The lines of `output` between the line `start` and the next line `end`.
fn block<'a>(output: &'a str, start: &str, end: &str) -> Vec<&'a str> {
    output
        .lines()
        .skip_while(|line| *line != start)
        .skip(1)
        .take_while(|line| *line != end)
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn generated_config_026_sonic_container_supervisord_runs_only_the_start_script() {
    let output = SonicLinuxContainerSupervisordTemplate {
        init_script: INIT_SCRIPT.to_owned(),
    }
    .render()
    .expect("template renders");
    assert_eq!(output, EXPECTED_SUPERVISORD);
}

#[test]
fn device_accounts_015_sonic_container_makes_the_owner_only_when_missing() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = init(Some(&hash), Some(CONFIG_DB));
    let account = block(&output, "if ! id alice >/dev/null 2>&1; then", "fi");
    assert!(!account.is_empty(), "{output}");
    let account = account.join("\n");
    assert!(account.contains("useradd -m -s /bin/bash -G frrvty alice"));
    assert!(account.contains(&format!("echo 'alice:{hash}' | chpasswd -e")));
    assert!(account.contains("ssh-rsa OWNERKEY1\nssh-ed25519 OWNERKEY2\n"));
    assert!(account.contains("install -d -m 700 -o alice -g alice /home/alice/.ssh"));
    assert!(account.contains("chmod 600 /home/alice/.ssh/authorized_keys"));
    assert!(account.contains("chown alice:alice /home/alice/.ssh/authorized_keys"));
    assert_eq!(output.matches("useradd").count(), 1);
    assert_eq!(output.matches("chpasswd").count(), 1);
}

#[test]
fn device_accounts_017_sonic_container_owner_gets_passwordless_sudo() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = init(Some(&hash), Some(CONFIG_DB));
    let with_sudo = block(&output, "if command -v sudo >/dev/null 2>&1; then", "else");
    assert_eq!(
        with_sudo,
        [
            "  echo 'alice ALL=(ALL) NOPASSWD: ALL' >/etc/sudoers.d/90-sherpa-owner",
            "  chmod 0440 /etc/sudoers.d/90-sherpa-owner",
        ]
    );
}

#[test]
fn device_accounts_018_sonic_container_without_sudo_gives_no_way_to_root() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = init(Some(&hash), Some(CONFIG_DB));
    let without_sudo = block(&output, "else", "fi");
    assert_eq!(
        without_sudo,
        [
            r#"  echo "sherpa-init: this image has no sudo, so alice has no root access: build the image with sudo""#
        ]
    );
    // No fallback: root gets no password and no `su`, and no fake `sudo` is written.
    assert!(!output.contains("root:"));
    assert!(!output.contains("su -"));
    assert!(!output.contains("/usr/local/bin/sudo"));
}

#[test]
fn device_accounts_016_sonic_container_locks_root_and_refuses_its_ssh_login() {
    for hash in [None, Some(helpers::sha512_hash("abcdefghijklm!@_-+."))] {
        let output = init(hash.as_deref(), Some(CONFIG_DB));
        assert!(output.contains("\nusermod -L root\n"), "{output}");
        assert!(output.contains("/usr/sbin/sshd -o PermitRootLogin=no\n"));
        assert!(!output.contains("-o 0") && !output.contains("-u 0"));
    }
}

#[test]
fn generated_config_027_sonic_container_waits_for_its_data_interfaces_then_hands_over() {
    let output = init(None, None);
    assert!(output.contains("DATA_COUNT=2\n"));
    assert!(output.contains("grep -c -E '^eth[1-9][0-9]*$'"));
    // A missing interface does not hold the node forever.
    assert!(output.contains("gave up waiting"));
    assert!(
        output
            .trim_end()
            .ends_with("exec /usr/local/bin/supervisord")
    );
}

#[test]
fn generated_config_028_sonic_container_loads_the_config_on_the_first_start_only() {
    let output = init(None, Some(CONFIG_DB));
    let load = block(
        &output,
        "  if [ ! -e /etc/sonic/.sherpa-config-loaded ]; then",
        "  fi",
    );
    assert_eq!(
        load,
        [
            "    sonic-cfggen -j /mnt/sherpa-ztp/config_db.json --write-to-db",
            "    sonic-cfggen -d --print-data >/etc/sonic/config_db.json",
            "    touch /etc/sonic/.sherpa-config-loaded",
        ]
    );
}

#[test]
fn generated_config_029_sonic_container_with_no_config_loads_none_and_makes_no_account() {
    let output = init(None, None);
    assert!(!output.contains("sonic-cfggen -j"));
    assert!(!output.contains(".sherpa-config-loaded"));
    assert!(!output.contains("useradd"));
    assert!(!output.contains("sudoers"));
    // The SSH server still starts last: it is the node's readiness signal.
    assert!(output.contains("/usr/sbin/sshd -o PermitRootLogin=no\n"));
}

#[test]
fn generated_config_030_sonic_container_starts_ssh_last() {
    let output = init(Some("HASH"), Some(CONFIG_DB));
    let initialized = output
        .find("CONFIG_DB_INITIALIZED")
        .expect("waits for the config database");
    let load = output.find("sonic-cfggen -j").expect("loads the config");
    let sshd = output.find("/usr/sbin/sshd").expect("starts sshd");
    let ready = output.find("sherpa-init: ready").expect("logs ready");
    assert!(initialized < load && load < sshd && sshd < ready);
}

#[test]
fn generated_config_031_sonic_container_makes_its_own_host_keys_once_before_ssh() {
    for (hash, config_db) in [(None, None), (Some("HASH"), Some(CONFIG_DB))] {
        let output = init(hash, config_db);
        let keys = block(
            &output,
            "if [ ! -e /etc/ssh/.sherpa-host-keys ]; then",
            "fi",
        );
        // The image's shared keys go, new ones are made, then the marker a restart finds.
        assert_eq!(
            keys,
            [
                "  rm -f /etc/ssh/ssh_host_*",
                "  ssh-keygen -A && touch /etc/ssh/.sherpa-host-keys",
            ],
            "{output}"
        );
        assert_eq!(output.matches("ssh-keygen").count(), 1);
        assert_eq!(output.matches("ssh_host_").count(), 1);
    }

    let output = init(Some("HASH"), Some(CONFIG_DB));
    let keygen = output.find("ssh-keygen -A").expect("makes host keys");
    let sshd = output.find("/usr/sbin/sshd").expect("starts sshd");
    let supervisord = output
        .find("exec /usr/local/bin/supervisord")
        .expect("hands over");
    assert!(keygen < sshd && keygen < supervisord, "{output}");
}

#[test]
fn node_vm_043_a_sonic_container_addresses_eth0_itself_before_its_network_os_starts() {
    let v4_only = init(None, None);
    let addresses = [
        "ip link set eth0 up",
        "ip addr replace 172.31.0.23/24 dev eth0",
        "ip route replace default via 172.31.0.1 dev eth0",
    ];
    for line in addresses {
        assert!(v4_only.lines().any(|l| l == line), "{line}\n{v4_only}");
    }
    assert!(!v4_only.contains("ip -6"));
    let supervisord = v4_only.find("exec /usr/local/bin/supervisord").unwrap();
    assert!(v4_only.find(addresses[1]).unwrap() < supervisord);

    let dual = SonicLinuxContainerInitTemplate {
        data_count: 2,
        owner: None,
        config_db: None,
        mgmt: management(Some(SonicLinuxContainerManagementV6 {
            address: "fd00:b0d1:0:1::17".parse::<Ipv6Addr>().unwrap(),
            prefix_length: 64,
            gateway: "fd00:b0d1:0:1::1".parse::<Ipv6Addr>().unwrap(),
        })),
    }
    .render()
    .unwrap();
    assert!(
        dual.lines()
            .any(|l| l == "ip -6 addr replace fd00:b0d1:0:1::17/64 dev eth0 nodad")
    );
    assert!(
        dual.lines()
            .any(|l| l == "ip -6 route replace default via fd00:b0d1:0:1::1 dev eth0")
    );
}
