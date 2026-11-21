use std::net::Ipv4Addr;

use askama::Template;

use template::{FrrStartupTemplate, FrrZtpTemplate};

use crate::helpers;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_FRR_CONFIG_STATIC: &str = "\
frr version 10
frr defaults traditional
hostname frr01
log syslog informational
service integrated-vtysh-config
!
interface eth0
 ip address 172.20.0.10/24
!
ip route 0.0.0.0/0 172.20.0.1
!
line vty
!";

const EXPECTED_FRR_STARTUP: &str = r#"#!/bin/sh
# Install OpenSSH server and sudo, retrying while the network is not yet reachable
for attempt in 1 2 3 4 5 6 7 8 9 10; do
  apk add --no-cache openssh-server sudo lldpd && break
  echo "apk add failed (attempt $attempt of 10), retrying in 3s"
  sleep 3
done
ssh-keygen -A

# Create vtysh config
touch /etc/frr/vtysh.conf
chown frr:frr /etc/frr/vtysh.conf

# Create the owner's account, with FRR group membership. It logs in by key, or with the owner's
# device password, set from its SHA-512-crypt hash.
adduser -D -s /bin/sh alice
addgroup alice frr
addgroup alice frrvty
echo 'alice:HASH' | chpasswd -e
echo 'alice ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/alice
mkdir -p /home/alice/.ssh
cat > /home/alice/.ssh/authorized_keys <<'SHERPA_KEYS'
ssh-rsa OWNERKEY1
ssh-ed25519 OWNERKEY2
SHERPA_KEYS
chmod 700 /home/alice/.ssh
chmod 600 /home/alice/.ssh/authorized_keys
chown -R alice:alice /home/alice/.ssh

# Set container hostname
hostname frr01
echo 'frr01' > /etc/hostname

# Start SSH daemon
/usr/sbin/sshd -D &

# Start LLDP daemon
lldpd

# Start FRR
/usr/lib/frr/frrinit.sh start

# Keep container running
tail -f /dev/null"#;

// ============================================================================
// Tests
// ============================================================================

#[test]
fn generated_config_016_frr_conf() {
    let t = FrrZtpTemplate {
        hostname: "frr01".to_string(),
        mgmt_ipv4: helpers::test_network_v4(),
        mgmt_ipv4_address: Some(Ipv4Addr::new(172, 20, 0, 10)),
        mgmt_ipv6_address: None,
        mgmt_ipv6: None,
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_FRR_CONFIG_STATIC);
}

fn startup(hash: &str) -> FrrStartupTemplate {
    FrrStartupTemplate {
        hostname: "frr01".to_string(),
        owner: helpers::test_owner(hash),
    }
}

#[test]
fn generated_config_017_frr_start_script() {
    let hash = helpers::sha512_hash("abcdefghijklm!@_-+.");
    let output = startup(&hash).render().expect("template renders");
    assert_eq!(output, EXPECTED_FRR_STARTUP.replace("HASH", &hash));
}
