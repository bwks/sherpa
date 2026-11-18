//! A node's serial console on a real host: the socket only sherpad reaches. Needs root (to add
//! and remove an OS account and run commands as other accounts), a libvirt daemon with KVM, the
//! installed `sherpa` account and `/usr/bin/python3`, so it runs in the VM release suite.
//!
//! A domain defined as Sherpa defined them before, with a telnet console on TCP, is moved to its
//! socket by the code `resume` and sherpad's startup use, in a folder made as sherpad makes a
//! lab's (`sherpa:kvm`, mode 2770, under `/opt/sherpa/run/console`), and started. Then nothing
//! listens on the loopback block the consoles used, `sherpa` opens the socket, and an OS account
//! in no groups cannot.

use std::fs;
use std::net::Ipv4Addr;
use std::process::{Command, Output};

use virt::connect::Connect;
use virt::domain::Domain;

use libvirt::{Qemu, SerialConsole, move_serial_to_socket, serial_console};

/// sherpad's console folder, the lab console folder it would make for lab `t674host`, and that
/// lab's node 1's socket.
const CONSOLE_ROOT: &str = "/opt/sherpa/run/console";
const CONSOLE_DIR: &str = "/opt/sherpa/run/console/t674host";
const SOCKET: &str = "/opt/sherpa/run/console/t674host/1.serial";
const DOMAIN: &str = "sherpa-test-console-socket";
/// An OS account the test adds in no group but `nogroup`, and removes.
const OTHER_ACCOUNT: &str = "sherpa-t674-other";
/// The loopback block the telnet consoles listened in, `127.127.0.0/16`.
const CONSOLE_LOOPBACK: [u8; 2] = [127, 127];
/// The state `/proc/net/tcp` gives a listening socket.
const TCP_LISTEN: &str = "0A";
/// Connects to the UNIX socket named by its first argument; fails with the error if it cannot.
const CONNECT: &str = "import socket, sys; socket.socket(socket.AF_UNIX).connect(sys.argv[1])";

/// Removes what the test made, however it ends.
struct Cleanup;

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(conn) = Qemu::default().connect() {
            remove_domain(&conn);
        }
        // The account may not exist; the test checks nothing about removing it.
        let _ = Command::new("userdel").arg(OTHER_ACCOUNT).output();
        let _ = fs::remove_dir_all(CONSOLE_DIR);
    }
}

#[test]
#[ignore = "requires root, a libvirt daemon with KVM, the sherpa account and /usr/bin/python3"]
fn a_console_socket_is_reached_by_sherpa_and_by_no_account_in_no_group() {
    let _cleanup = Cleanup;
    let conn = Qemu::default().connect().expect("connects to libvirt");
    remove_domain(&conn);
    let _ = Command::new("userdel").arg(OTHER_ACCOUNT).output();

    make_console_dir();
    let path = SOCKET.to_owned();
    let domain = Domain::define_xml(&conn, &tcp_console_domain()).expect("defines the domain");
    assert!(move_serial_to_socket(&conn, DOMAIN, &path).expect("moves the console"));
    assert!(!move_serial_to_socket(&conn, DOMAIN, &path).expect("reads the domain again"));
    domain.create().expect("starts the domain");
    assert_eq!(
        serial_console(&conn, DOMAIN).expect("reads the running domain"),
        Some(SerialConsole::Socket(path.clone()))
    );

    let listening = loopback_console_listeners(
        &fs::read_to_string("/proc/net/tcp").expect("reads the TCP sockets"),
    );
    assert!(
        listening.is_empty(),
        "listening in 127.127.0.0/16: {listening:?}"
    );

    let sherpa = run_as(
        &["--reuid=sherpa", "--regid=sherpa", "--init-groups"],
        &path,
    );
    assert!(
        sherpa.status.success(),
        "sherpa cannot open the socket: {}",
        String::from_utf8_lossy(&sherpa.stderr)
    );

    let added = Command::new("useradd")
        .args([
            "--system",
            "--no-create-home",
            "--no-user-group",
            "--gid",
            "nogroup",
        ])
        .args(["--shell", "/usr/sbin/nologin", OTHER_ACCOUNT])
        .output()
        .expect("runs useradd");
    assert!(added.status.success(), "{added:?}");
    let reuid = format!("--reuid={OTHER_ACCOUNT}");
    let other = run_as(&[&reuid, "--regid=nogroup", "--clear-groups"], &path);
    let refusal = String::from_utf8_lossy(&other.stderr);
    assert!(!other.status.success(), "{OTHER_ACCOUNT} opened the socket");
    assert!(refusal.contains("PermissionError"), "{refusal}");
}

/// The lab's console folder, and the folder above it, as sherpad makes them: `sherpa:kvm`, modes
/// 2750 and 2770. The folder above is made as sherpad would, so a test run before sherpad's
/// first start does not leave it to root.
fn make_console_dir() {
    for (mode, dir) in [("2750", CONSOLE_ROOT), ("2770", CONSOLE_DIR)] {
        let status = Command::new("install")
            .args(["-d", "-o", "sherpa", "-g", "kvm", "-m", mode, dir])
            .status()
            .expect("runs install");
        assert!(status.success(), "install -d {dir}");
    }
}

/// A 64 MiB domain with no disks and a telnet console on TCP, as Sherpa defined domains before
/// consoles moved behind sherpad.
fn tcp_console_domain() -> String {
    format!(
        "<domain type='kvm'><name>{DOMAIN}</name><memory unit='MiB'>64</memory><vcpu>1</vcpu>\
         <os><type arch='x86_64'>hvm</type></os><devices>\
         <serial type='tcp'><source mode='bind' host='127.127.250.1' service='2323'/>\
         <protocol type='telnet'/><target port='0'/></serial>\
         <console type='tcp'><source mode='bind' host='127.127.250.1' service='2323'/>\
         <protocol type='telnet'/><target type='serial' port='0'/></console>\
         </devices></domain>"
    )
}

/// Connect to the socket at `path` as the account `setpriv` sets with `identity`.
fn run_as(identity: &[&str], path: &str) -> Output {
    Command::new("setpriv")
        .args(identity)
        .args(["/usr/bin/python3", "-I", "-c", CONNECT, path])
        .output()
        .expect("runs setpriv")
}

/// Stop and remove the test domain, if it is there.
fn remove_domain(conn: &Connect) {
    if let Ok(domain) = Domain::lookup_by_name(conn, DOMAIN) {
        let _ = domain.destroy();
        let _ = domain.undefine();
    }
}

/// The addresses in `127.127.0.0/16` listening for TCP, from `/proc/net/tcp` as `table`.
fn loopback_console_listeners(table: &str) -> Vec<Ipv4Addr> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace().skip(1);
            let local = fields.next()?;
            let state = fields.nth(1)?;
            let address = u32::from_str_radix(local.split(':').next()?, 16).ok()?;
            // The kernel writes the address in host order, little-endian here.
            let address = Ipv4Addr::from(address.to_le_bytes());
            (state == TCP_LISTEN && address.octets()[..2] == CONSOLE_LOOPBACK).then_some(address)
        })
        .collect()
}

#[test]
fn only_listeners_in_the_console_loopback_block_are_found() {
    let table = "  sl  local_address rem_address   st tx_queue rx_queue\n\
        0: 01FA7F7F:0913 00000000:0000 0A 00000000:00000000\n\
        1: 0100007F:0CEA 00000000:0000 0A 00000000:00000000\n\
        2: 02FA7F7F:0913 0100007F:9C40 01 00000000:00000000\n";
    assert_eq!(
        loopback_console_listeners(table),
        [Ipv4Addr::new(127, 127, 250, 1)]
    );
}
