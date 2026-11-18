use askama::Template;

use template::{SonicLinuxContainerInitTemplate, SonicLinuxContainerSupervisordTemplate};

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

fn init(hash: Option<&str>, config_db: Option<&str>) -> String {
    SonicLinuxContainerInitTemplate {
        data_count: 2,
        owner: hash.map(helpers::test_owner),
        config_db: config_db.map(str::to_owned),
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
fn test_supervisord_runs_only_the_init_script() {
    let output = SonicLinuxContainerSupervisordTemplate {
        init_script: INIT_SCRIPT.to_owned(),
    }
    .render()
    .expect("template renders");
    assert_eq!(output, EXPECTED_SUPERVISORD);
}

#[test]
fn test_init_makes_the_owner_account_only_when_it_does_not_exist() {
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
    helpers::assert_no_sherpa_login(&output);
}

#[test]
fn test_init_gives_the_owner_passwordless_sudo_when_the_image_has_sudo() {
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
fn test_init_logs_a_line_and_gives_no_root_path_when_the_image_has_no_sudo() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = init(Some(&hash), Some(CONFIG_DB));
    let without_sudo = block(&output, "else", "fi");
    assert_eq!(
        without_sudo,
        [
            "  echo \"sherpa-init: this image has no sudo, so alice has no root access: \
          build the image with sudo\""
        ]
    );
    // No fallback: root gets no password and no `su`, and no fake `sudo` is written.
    assert!(!output.contains("root:"));
    assert!(!output.contains("su -"));
    assert!(!output.contains("/usr/local/bin/sudo"));
}

#[test]
fn test_init_locks_root_and_refuses_its_ssh_login() {
    for hash in [None, Some(helpers::sha512_hash("abcdefghijklm!@_-+."))] {
        let output = init(hash.as_deref(), Some(CONFIG_DB));
        assert!(output.contains("\nusermod -L root\n"), "{output}");
        assert!(output.contains("/usr/sbin/sshd -o PermitRootLogin=no\n"));
        assert!(!output.contains("-o 0") && !output.contains("-u 0"));
    }
}

#[test]
fn test_init_links_rvtysh_to_vtysh() {
    let output = init(None, None);
    assert!(
        output.contains(
            "[ -e /usr/local/bin/rvtysh ] || ln -s /usr/bin/vtysh /usr/local/bin/rvtysh\n"
        )
    );
}

#[test]
fn test_init_password_is_a_hash_that_verifies_and_never_plain_text() {
    for password in helpers::ALL_CHARACTER_PASSWORDS {
        let hash = helpers::sha512_hash(password);
        let output = init(Some(&hash), Some(CONFIG_DB));
        let found = helpers::hash_after(&output, "alice:");
        assert_eq!(found, hash);
        helpers::assert_hash_verifies(&found, "$6$", password);
        assert!(!output.contains(password));
    }
}

#[test]
fn test_init_waits_for_the_data_interfaces_then_hands_over_to_sonic() {
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
fn test_init_loads_sherpas_config_on_the_first_start_only_and_saves_it() {
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
fn test_init_with_no_config_loads_none_and_makes_no_account() {
    let output = init(None, None);
    assert!(!output.contains("sonic-cfggen -j"));
    assert!(!output.contains(".sherpa-config-loaded"));
    assert!(!output.contains("useradd"));
    assert!(!output.contains("sudoers"));
    // The SSH server still starts last: it is the node's readiness signal.
    assert!(output.contains("/usr/sbin/sshd -o PermitRootLogin=no\n"));
}

#[test]
fn test_init_starts_ssh_last_after_the_switch_and_its_config() {
    let output = init(Some("HASH"), Some(CONFIG_DB));
    let initialized = output
        .find("CONFIG_DB_INITIALIZED")
        .expect("waits for the config database");
    let load = output.find("sonic-cfggen -j").expect("loads the config");
    let sshd = output.find("/usr/sbin/sshd").expect("starts sshd");
    let ready = output.find("sherpa-init: ready").expect("logs ready");
    assert!(initialized < load && load < sshd && sshd < ready);
    assert!(output.contains("sysctl -w net.ipv6.conf.all.forwarding=1"));
    assert!(output.contains("supervisorctl start bgpd"));
}
