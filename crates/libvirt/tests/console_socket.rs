//! A node's serial and VNC consoles on a real host: the sockets only sherpad reaches. Needs root (to add
//! and remove an OS account and run commands as other accounts), a libvirt daemon with KVM, the
//! installed `sherpa` account and `/usr/bin/python3`, so it runs in the VM release suite.
//!
//! Each test defines a domain as Sherpa defined them before, with a telnet console and VNC on TCP,
//! moves it to its sockets by the code `resume` and sherpad's startup use, in a folder made as
//! sherpad makes a lab's (`sherpa:kvm`, mode 2770, under `/opt/sherpa/run/console`), and starts
//! it. Each test has its own lab ID, so its domain and folder are its own.

use std::fs;
use std::net::Ipv4Addr;
use std::process::{Command, Output};

use virt::connect::Connect;
use virt::domain::Domain;

use libvirt::{
    ConsoleSockets, Qemu, SerialConsole, VncConsole, move_consoles_to_sockets, serial_console,
    vnc_console,
};

/// sherpad's console folder.
const CONSOLE_ROOT: &str = "/opt/sherpa/run/console";
/// An OS account the test adds in no group but `nogroup`, and removes.
const OTHER_ACCOUNT: &str = "sherpa-t674-other";
/// The loopback block the telnet consoles listened in, `127.127.0.0/16`.
const CONSOLE_LOOPBACK: [u8; 2] = [127, 127];
/// The ports VNC consoles listened on, before they moved to sockets.
const VNC_PORTS: std::ops::RangeInclusive<u16> = 5900..=6999;
/// The state `/proc/net/tcp` gives a listening socket.
const TCP_LISTEN: &str = "0A";
/// Connects to the UNIX socket named by its first argument; fails with the error if it cannot.
const CONNECT: &str = "import socket, sys; socket.socket(socket.AF_UNIX).connect(sys.argv[1])";

/// A test's lab: its console folder, node 1's sockets and the domain, all named by the lab ID.
/// Dropping it removes the domain and the folder, however the test ends.
struct TestLab {
    dir: String,
    sockets: ConsoleSockets,
    domain: String,
}

/// Removes the OS account [`OTHER_ACCOUNT`], however the test ends.
struct OtherAccount;

impl TestLab {
    /// A lab named by `lab_id`, with nothing of it left from an earlier run.
    fn new(conn: &Connect, lab_id: &str) -> Self {
        let dir = format!("{CONSOLE_ROOT}/{lab_id}");
        let lab = Self {
            sockets: ConsoleSockets {
                serial: format!("{dir}/1.serial"),
                vnc: format!("{dir}/1.vnc"),
            },
            domain: format!("sherpa-test-console-{lab_id}"),
            dir,
        };
        remove_domain(conn, &lab.domain);
        lab
    }

    /// Makes the lab's console folder, defines the domain with its consoles on TCP and returns it.
    fn define(&self, conn: &Connect) -> Domain {
        make_console_dir(&self.dir);
        Domain::define_xml(conn, &tcp_console_domain(&self.domain)).expect("defines the domain")
    }

    /// Defines the domain, moves its consoles to their sockets and starts it.
    fn start_moved(&self, conn: &Connect) {
        let domain = self.define(conn);
        move_consoles_to_sockets(conn, &self.domain, &self.sockets).expect("moves the consoles");
        domain.create().expect("starts the domain");
    }
}

impl Drop for TestLab {
    fn drop(&mut self) {
        if let Ok(conn) = Qemu::default().connect() {
            remove_domain(&conn, &self.domain);
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Drop for OtherAccount {
    fn drop(&mut self) {
        // The account may not exist; the test checks nothing about removing it.
        let _ = Command::new("userdel").arg(OTHER_ACCOUNT).output();
    }
}

#[test]
#[ignore = "requires root, a libvirt daemon with KVM, the sherpa account and /usr/bin/python3"]
fn console_upgrade_009_a_domain_moved_then_started_runs_with_both_consoles_on_its_sockets() {
    let conn = Qemu::default().connect().expect("connects to libvirt");
    let lab = TestLab::new(&conn, "t674upgr");
    let domain = lab.define(&conn);
    assert!(
        move_consoles_to_sockets(&conn, &lab.domain, &lab.sockets).expect("moves the consoles")
    );
    assert!(
        !move_consoles_to_sockets(&conn, &lab.domain, &lab.sockets)
            .expect("reads the domain again")
    );
    domain.create().expect("starts the domain");
    assert_eq!(
        serial_console(&conn, &lab.domain).expect("reads the running domain"),
        Some(SerialConsole::Socket(lab.sockets.serial.clone()))
    );
    assert_eq!(
        vnc_console(&conn, &lab.domain).expect("reads the running domain"),
        Some(VncConsole::Socket(lab.sockets.vnc.clone()))
    );
}

#[test]
#[ignore = "requires root, a libvirt daemon with KVM, the sherpa account and /usr/bin/python3"]
fn console_sockets_002_a_running_nodes_consoles_listen_on_no_tcp_port() {
    let conn = Qemu::default().connect().expect("connects to libvirt");
    let lab = TestLab::new(&conn, "t674ntcp");
    lab.start_moved(&conn);

    let table = fs::read_to_string("/proc/net/tcp").expect("reads the TCP sockets");
    let listening = loopback_console_listeners(&table);
    assert!(
        listening.is_empty(),
        "listening in 127.127.0.0/16: {listening:?}"
    );
    let vnc_ports = vnc_listeners(&table);
    assert!(
        vnc_ports.is_empty(),
        "listening on VNC ports: {vnc_ports:?}"
    );
}

#[test]
#[ignore = "requires root, a libvirt daemon with KVM, the sherpa account and /usr/bin/python3"]
fn console_sockets_001_sherpa_opens_both_sockets_and_an_account_in_neither_group_is_refused() {
    let _account = OtherAccount;
    let conn = Qemu::default().connect().expect("connects to libvirt");
    let lab = TestLab::new(&conn, "t674perm");
    let _ = Command::new("userdel").arg(OTHER_ACCOUNT).output();
    lab.start_moved(&conn);

    let sockets = [lab.sockets.serial.as_str(), lab.sockets.vnc.as_str()];
    for socket in sockets {
        let sherpa = run_as(
            &["--reuid=sherpa", "--regid=sherpa", "--init-groups"],
            socket,
        );
        assert!(
            sherpa.status.success(),
            "sherpa cannot open {socket}: {}",
            String::from_utf8_lossy(&sherpa.stderr)
        );
    }

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
    for socket in sockets {
        let other = run_as(&[&reuid, "--regid=nogroup", "--clear-groups"], socket);
        let refusal = String::from_utf8_lossy(&other.stderr);
        assert!(!other.status.success(), "{OTHER_ACCOUNT} opened {socket}");
        assert!(refusal.contains("PermissionError"), "{refusal}");
    }
}

/// The lab's console folder `dir`, and the folder above it, as sherpad makes them: `sherpa:kvm`,
/// modes 2750 and 2770. The folder above is made as sherpad would, so a test run before sherpad's
/// first start does not leave it to root.
fn make_console_dir(dir: &str) {
    for (mode, dir) in [("2750", CONSOLE_ROOT), ("2770", dir)] {
        let status = Command::new("install")
            .args(["-d", "-o", "sherpa", "-g", "kvm", "-m", mode, dir])
            .status()
            .expect("runs install");
        assert!(status.success(), "install -d {dir}");
    }
}

/// A 64 MiB domain with no disks, a telnet console and VNC on TCP, as Sherpa defined domains
/// before consoles moved behind sherpad.
fn tcp_console_domain(name: &str) -> String {
    format!(
        "<domain type='kvm'><name>{name}</name><memory unit='MiB'>64</memory><vcpu>1</vcpu>\
         <os><type arch='x86_64'>hvm</type></os><devices>\
         <serial type='tcp'><source mode='bind' host='127.127.250.1' service='2323'/>\
         <protocol type='telnet'/><target port='0'/></serial>\
         <console type='tcp'><source mode='bind' host='127.127.250.1' service='2323'/>\
         <protocol type='telnet'/><target type='serial' port='0'/></console>\
         <graphics type='vnc' port='6999' autoport='no'/>\
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

/// Stop and remove the test domain `name`, if it is there.
fn remove_domain(conn: &Connect, name: &str) {
    if let Ok(domain) = Domain::lookup_by_name(conn, name) {
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

/// The ports in [`VNC_PORTS`] listening for TCP on any address, from `/proc/net/tcp` as `table`.
fn vnc_listeners(table: &str) -> Vec<u16> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace().skip(1);
            let local = fields.next()?;
            let state = fields.nth(1)?;
            let port = u16::from_str_radix(local.split(':').nth(1)?, 16).ok()?;
            (state == TCP_LISTEN && VNC_PORTS.contains(&port)).then_some(port)
        })
        .collect()
}

#[test]
fn only_listeners_on_vnc_ports_are_found() {
    let table = "  sl  local_address rem_address   st tx_queue rx_queue\n\
        0: 0100007F:1B57 00000000:0000 0A 00000000:00000000\n\
        1: 0100007F:1B58 00000000:0000 0A 00000000:00000000\n\
        2: 0100007F:170C 0100007F:9C40 01 00000000:00000000\n";
    assert_eq!(vnc_listeners(table), [6999]);
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
