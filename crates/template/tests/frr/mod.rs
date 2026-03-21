use std::net::Ipv4Addr;

use askama::Template;

use template::{FrrDaemonsTemplate, FrrStartupTemplate, FrrZtpTemplate};

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

const EXPECTED_FRR_DAEMONS: &str = "\
zebra=yes
bgpd=no
ospfd=no
ospf6d=no
ripd=no
ripngd=no
isisd=no
pimd=no
ldpd=no
nhrpd=no
eigrpd=no
babeld=no
sharpd=no
pbrd=no
vrrpd=no
staticd=no

vtysh_enable=yes";

const EXPECTED_FRR_STARTUP: &str = "\
#!/bin/sh
# Install OpenSSH server and sudo
apk add --no-cache openssh-server sudo lldpd
ssh-keygen -A

# Create vtysh config
touch /etc/frr/vtysh.conf
chown frr:frr /etc/frr/vtysh.conf

# Create sherpa user with SSH access and FRR group membership
adduser -D -s /bin/sh sherpa
addgroup sherpa frr
addgroup sherpa frrvty
echo 'sherpa:Everest1953!' | chpasswd
echo 'sherpa ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/sherpa
mkdir -p /home/sherpa/.ssh
echo 'ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ' > /home/sherpa/.ssh/authorized_keys
chmod 700 /home/sherpa/.ssh
chmod 600 /home/sherpa/.ssh/authorized_keys
chown -R sherpa:sherpa /home/sherpa/.ssh

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
tail -f /dev/null";

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_frr_config_static_ipv4() {
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

#[test]
fn test_frr_daemons() {
    let t = FrrDaemonsTemplate {};
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_FRR_DAEMONS);
}

#[test]
fn test_frr_startup() {
    let t = FrrStartupTemplate {
        hostname: "frr01".to_string(),
        user: helpers::test_user(),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_FRR_STARTUP);
}
