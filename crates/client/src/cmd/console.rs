//! `sherpa console`: a node's serial console in the local terminal. Each VM and unikernel has
//! a QEMU telnet chardev on its address in the lab's loopback network on the Sherpa server,
//! so the console is always reached through an SSH session to the server, the hop `sherpa ssh`
//! takes, with Jato's telnet client over a `direct-tcpip` channel. No system `telnet` is used.
//! The node's kind and console address come from the server's `inspect`, so a lab folder
//! needs no manifest; a server too old to give the address falls back to the node's place in
//! the folder's `manifest.toml`.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use jato::{
    ConnectOptions, Error, InteractiveOptions, Node, PromptCache, SshConfig, Telnet, TelnetOptions,
};

use shared::data::{ClientConfig, DeviceInfo, LabInfo, NodeKind};
use shared::konst::{
    BOOT_SERVER_NAME, CONSOLE_CONNECT_TIMEOUT, CONSOLE_NEGOTIATION_TIMEOUT, SHERPA_MANIFEST_FILE,
    SSH_PROMPT_AUTH_TIMEOUT, TELNET_PORT,
};
use shared::util::{LabFolder, get_ip, term_msg_surround};
use topology::Manifest;

use super::inspect::fetch_inspect;
use super::ssh::read_lab_connection_file;
use crate::ssh_client;

/// Open the serial console of the node `name` in this process's terminal, through the Sherpa
/// server, until the console closes or an escape (`Ctrl+]`, or `~.` at the start of a line) is
/// typed. Containers have no serial console and are refused before anything connects.
#[tracing::instrument(skip_all, fields(%name, %lab_id))]
pub async fn console(
    name: &str,
    lab_id: &str,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    let inspect = fetch_inspect(lab_id, server_url, config).await?;
    let lab_info = &inspect.lab_info;
    let node = match console_target(name, &inspect.devices, lab_info)? {
        Some(node) => node,
        None => manifest_console_node(name, Path::new(SHERPA_MANIFEST_FILE), lab_info)?,
    };
    let lab_dir = Path::new("");
    let folder = LabFolder::new(lab_dir);
    let ssh_config = read_lab_connection_file(&folder.ssh_config(), &lab_info.id)?;
    let alias = first_host_alias(&ssh_config).ok_or_else(|| {
        anyhow!(
            "The lab SSH config names no node to find the Sherpa server by. Restore the lab \
             files with 'sherpa download {} --force'.",
            lab_info.id
        )
    })?;
    let route = SshConfig::load_file(folder.ssh_config())
        .context("Failed to read the lab SSH config")?
        .route(alias)
        .with_context(|| format!("Failed to resolve '{alias}' in the lab SSH config"))?;
    term_msg_surround(&format!("Connecting to: {name}"));

    let mut options = ConnectOptions::default();
    options.auth_timeout = Duration::from_secs(SSH_PROMPT_AUTH_TIMEOUT);
    let session = ssh_client::connect_server(&route, &options, &PromptCache::new()).await?;
    let mut telnet_options = TelnetOptions::default();
    telnet_options.connect_timeout = Duration::from_secs(CONSOLE_CONNECT_TIMEOUT);
    telnet_options.negotiation_timeout = Some(Duration::from_secs(CONSOLE_NEGOTIATION_TIMEOUT));
    let mut telnet = Telnet::connect_via(&session, &node, &telnet_options)
        .await
        .map_err(|error| console_error(name, error))?;

    eprintln!("{}", connected_line(name, &lab_info.name));
    // Sherpa prints its own lines, with the node's name, in place of Jato's.
    let mut interactive = InteractiveOptions::default();
    interactive.banner = false;
    let end = telnet
        .interactive(&interactive)
        .await
        .map_err(|error| console_error(name, error))?;
    drop(telnet);
    if let Err(error) = session.close().await {
        tracing::debug!(%error, "Failed to close the SSH session to the server cleanly");
    }
    eprintln!("{}", closed_line(name, end.escaped));
    Ok(())
}

/// The console of `name` on the Sherpa server, from the lab's `devices` as `inspect` gives
/// them: the node's console address on the telnet port. The boot server's is host 255 of the
/// lab's loopback network. A container has none and is refused. `None` when the server gives
/// no address, as servers before console addresses do.
fn console_target(name: &str, devices: &[DeviceInfo], lab_info: &LabInfo) -> Result<Option<Node>> {
    if name == BOOT_SERVER_NAME {
        let address = get_ip(&lab_info.loopback_network, 255);
        return Ok(Some(Node::new(address.to_string(), TELNET_PORT)));
    }
    let device = devices
        .iter()
        .find(|device| device.name == name)
        .ok_or_else(|| anyhow!("Device not found: {name}"))?;
    if device.kind == NodeKind::Container {
        bail!(
            "{name} is a container, and containers have no serial console. Connect with \
             'sherpa ssh {name}' instead."
        );
    }
    Ok(device
        .console_address
        .map(|address| Node::new(address.to_string(), TELNET_PORT)))
}

/// The console of `name` from its place in the manifest at `path`, for a server that gives no
/// console address: node indexes follow the manifest's order, starting from 1, as `up` gives
/// them.
fn manifest_console_node(name: &str, path: &Path, lab_info: &LabInfo) -> Result<Node> {
    if !path.exists() {
        bail!(
            "The Sherpa server did not give the console address of {name}: it is older than \
             this client. Upgrade the server, or run 'sherpa console' in the folder the lab \
             was started from, with its {SHERPA_MANIFEST_FILE}."
        );
    }
    let manifest = Manifest::load_file(&path.to_string_lossy())?;
    let position = manifest
        .nodes
        .iter()
        .position(|device| device.name == name)
        .ok_or_else(|| anyhow!("Device not found: {name}"))?;
    let device_id =
        u8::try_from(position + 1).with_context(|| format!("Device ID not found for: {name}"))?;
    Ok(Node::new(
        get_ip(&lab_info.loopback_network, device_id).to_string(),
        TELNET_PORT,
    ))
}

/// The first alias of the first `Host` block in the lab SSH config. Every block jumps through
/// the Sherpa server, so any node's route names the server hop.
fn first_host_alias(ssh_config: &str) -> Option<&str> {
    ssh_config.lines().find_map(|line| {
        let mut fields = line.split_ascii_whitespace();
        let directive = fields.next()?;
        if directive.eq_ignore_ascii_case("Host") {
            fields.next().filter(|alias| !alias.starts_with('#'))
        } else {
            None
        }
    })
}

/// What the user is told when the console of `name` cannot be opened or fails.
fn console_error(name: &str, error: Error) -> anyhow::Error {
    let message = match &error {
        Error::TelnetRefused { node, .. } => format!(
            "The console of {name} refused the connection at {node} through the Sherpa \
             server. Is the node running?"
        ),
        // QEMU listens with a backlog of one and accepts nobody while a client is attached,
        // so once two tries are queued behind the holder, a new one is not answered at all.
        Error::TelnetTimeout { node, after, .. } => format!(
            "The console of {name} did not answer at {node} within {}s. Another client may \
             hold it: while one does, the console takes only two more connections and then \
             none until it leaves. Close the other session and try again, or check the node \
             is running with 'sherpa inspect'.",
            after.as_secs()
        ),
        Error::TelnetInUse { after, .. } => format!(
            "The console of {name} is in use: another client holds it, and it said nothing \
             within {}s. A console serves one client at a time; close the other session and \
             try again.",
            after.as_secs()
        ),
        Error::TelnetClosed { node, .. } => {
            format!("The console of {name} closed the connection at {node}. Is the node running?")
        }
        _ => format!("Failed to open the console of {name} through the Sherpa server"),
    };
    anyhow::Error::new(error).context(message)
}

/// The line on stderr once the console is open, naming the node, its lab and both escapes.
fn connected_line(name: &str, lab_name: &str) -> String {
    format!(
        "Connected to the console of {name} in lab {lab_name}. Escape: Ctrl+] or ~. at the \
         start of a line."
    )
}

/// The line on stderr when the console session ends. After an escape the cursor is usually on
/// the node's prompt, so the line starts on a new line.
fn closed_line(name: &str, escaped: bool) -> String {
    let gap = if escaped { "\n" } else { "" };
    format!("{gap}Connection to the console of {name} closed.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io;
    use std::time::Duration;

    use shared::data::{NodeModel, NodeState};

    fn lab_info() -> LabInfo {
        "id='abcd1234'\nname='test'\nuser='alice'\nipv4_network='192.0.2.0/24'\nipv4_gateway='192.0.2.1'\nipv4_router='192.0.2.2'\nloopback_network='127.127.3.0/24'".parse().unwrap()
    }

    fn console() -> Node {
        Node::new("127.127.3.1", TELNET_PORT)
    }

    fn server() -> Node {
        Node::new("192.0.2.1", 22)
    }

    fn device(name: &str, kind: NodeKind, console_address: Option<&str>) -> DeviceInfo {
        DeviceInfo {
            name: name.to_string(),
            model: NodeModel::AlpineLinux,
            kind,
            state: NodeState::Running,
            mgmt_ipv4: "192.0.2.10".to_string(),
            mgmt_ipv6: None,
            vnc_port: None,
            disks: Vec::new(),
            cpu_count: None,
            memory: None,
            console_address: console_address.map(|address| address.parse().unwrap()),
        }
    }

    /// The lab's nodes as a server with console addresses gives them. The VM's address is not
    /// the one its place in a manifest would give.
    fn devices() -> Vec<DeviceInfo> {
        vec![
            device("srl01", NodeKind::Container, None),
            device("alp01", NodeKind::VirtualMachine, Some("127.127.3.7")),
        ]
    }

    #[test]
    fn a_vm_console_is_at_the_address_the_server_gives() {
        assert_eq!(
            console_target("alp01", &devices(), &lab_info()).unwrap(),
            Some(Node::new("127.127.3.7", TELNET_PORT))
        );
    }

    #[test]
    fn the_boot_server_console_is_host_255_of_the_loopback_network() {
        assert_eq!(
            console_target(BOOT_SERVER_NAME, &[], &lab_info()).unwrap(),
            Some(Node::new("127.127.3.255", TELNET_PORT))
        );
    }

    #[test]
    fn a_container_is_refused_before_anything_connects() {
        let error = console_target("srl01", &devices(), &lab_info()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "srl01 is a container, and containers have no serial console. Connect with \
             'sherpa ssh srl01' instead."
        );
    }

    #[test]
    fn a_node_not_in_the_lab_is_reported() {
        let error = console_target("nope01", &devices(), &lab_info()).unwrap_err();
        assert_eq!(error.to_string(), "Device not found: nope01");
    }

    #[test]
    fn an_older_server_gives_no_console_address() {
        let devices = [device("alp01", NodeKind::VirtualMachine, None)];
        assert_eq!(
            console_target("alp01", &devices, &lab_info()).unwrap(),
            None
        );
    }

    #[test]
    fn with_an_older_server_the_console_is_found_by_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SHERPA_MANIFEST_FILE);
        fs::write(
            &path,
            "name='test'\nnodes=[{name='srl01',model='nokia_srlinux'},{name='alp01',model='alpine_linux'}]\n",
        )
        .unwrap();
        assert_eq!(
            manifest_console_node("alp01", &path, &lab_info()).unwrap(),
            Node::new("127.127.3.2", TELNET_PORT)
        );
        let error = manifest_console_node("nope01", &path, &lab_info()).unwrap_err();
        assert_eq!(error.to_string(), "Device not found: nope01");
    }

    #[test]
    fn with_an_older_server_and_no_manifest_the_server_is_too_old() {
        let dir = tempfile::tempdir().unwrap();
        let error =
            manifest_console_node("alp01", &dir.path().join(SHERPA_MANIFEST_FILE), &lab_info())
                .unwrap_err();
        assert_eq!(
            error.to_string(),
            "The Sherpa server did not give the console address of alp01: it is older than \
             this client. Upgrade the server, or run 'sherpa console' in the folder the lab \
             was started from, with its manifest.toml."
        );
    }

    #[test]
    fn the_first_host_alias_is_the_way_to_the_server() {
        let config = "# Host comment\nHost 192.0.2.10 vm.abcd1234 vm.abcd1234.sherpa.lab.local\n    HostName 192.0.2.10\n    ProxyJump alice@192.0.2.1\nHost 192.0.2.11\n";
        assert_eq!(first_host_alias(config), Some("192.0.2.10"));
        assert_eq!(first_host_alias("HostName 192.0.2.10\n"), None);
        assert_eq!(first_host_alias(""), None);
    }

    #[test]
    fn a_refused_console_asks_whether_the_node_is_running() {
        let error = console_error(
            "alp01",
            Error::TelnetRefused {
                node: console(),
                via: Some(server()),
                source: Box::new(io::Error::other("connect failed")),
            },
        );
        assert_eq!(
            error.to_string(),
            "The console of alp01 refused the connection at 127.127.3.1:2323 through the \
             Sherpa server. Is the node running?"
        );
    }

    #[test]
    fn a_console_that_does_not_answer_gives_the_time_limit() {
        let error = console_error(
            "alp01",
            Error::TelnetTimeout {
                node: console(),
                via: Some(server()),
                after: Duration::from_secs(CONSOLE_CONNECT_TIMEOUT),
            },
        );
        assert_eq!(
            error.to_string(),
            "The console of alp01 did not answer at 127.127.3.1:2323 within 10s. Another \
             client may hold it: while one does, the console takes only two more connections \
             and then none until it leaves. Close the other session and try again, or check \
             the node is running with 'sherpa inspect'."
        );
    }

    #[test]
    fn a_silent_console_is_held_by_another_client() {
        let error = console_error(
            "alp01",
            Error::TelnetInUse {
                node: console(),
                after: Duration::from_secs(CONSOLE_NEGOTIATION_TIMEOUT),
            },
        );
        assert_eq!(
            error.to_string(),
            "The console of alp01 is in use: another client holds it, and it said nothing \
             within 3s. A console serves one client at a time; close the other session and \
             try again."
        );
    }

    #[test]
    fn a_closed_console_is_reported() {
        let error = console_error(
            "alp01",
            Error::TelnetClosed {
                node: console(),
                source: io::Error::from(io::ErrorKind::UnexpectedEof),
            },
        );
        assert_eq!(
            error.to_string(),
            "The console of alp01 closed the connection at 127.127.3.1:2323. Is the node \
             running?"
        );
    }

    #[test]
    fn other_errors_keep_their_cause() {
        let error = console_error(
            "alp01",
            Error::Tunnel {
                via: server(),
                node: console(),
                source: Box::new(io::Error::other("connection reset")),
            },
        );
        assert_eq!(
            error.to_string(),
            "Failed to open the console of alp01 through the Sherpa server"
        );
        assert!(error.downcast_ref::<Error>().is_some());
    }

    #[test]
    fn the_connect_line_names_the_node_the_lab_and_both_escapes() {
        assert_eq!(
            connected_line("alp01", "test"),
            "Connected to the console of alp01 in lab test. Escape: Ctrl+] or ~. at the \
             start of a line."
        );
    }

    #[test]
    fn the_close_line_starts_on_a_new_line_after_an_escape() {
        assert_eq!(
            closed_line("alp01", false),
            "Connection to the console of alp01 closed."
        );
        assert_eq!(
            closed_line("alp01", true),
            "\nConnection to the console of alp01 closed."
        );
    }
}
