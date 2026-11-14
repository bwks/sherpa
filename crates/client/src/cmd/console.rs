//! `sherpa console`: a node's serial console in the local terminal. Each VM and unikernel has
//! a QEMU telnet chardev on its address in the lab's loopback network on the Sherpa server,
//! so the console is always reached through an SSH session to the server, the hop `sherpa ssh`
//! takes, with Jato's telnet client over a `direct-tcpip` channel. No system `telnet` is used.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use jato::{
    ConnectOptions, Error, InteractiveOptions, Node, PromptCache, SshConfig, Telnet, TelnetOptions,
};

use shared::data::{LabInfo, NodeKind};
use shared::konst::{
    BOOT_SERVER_NAME, CONSOLE_CONNECT_TIMEOUT, CONSOLE_NEGOTIATION_TIMEOUT,
    SSH_PROMPT_AUTH_TIMEOUT, TELNET_PORT,
};
use shared::util::{LabFolder, get_ip, term_msg_surround};
use topology::Manifest;

use super::ssh::read_lab_connection_file;
use crate::ssh_client;

/// Open the serial console of the node `name` in this process's terminal, through the Sherpa
/// server, until the console closes or an escape (`Ctrl+]`, or `~.` at the start of a line) is
/// typed. Containers have no serial console and are refused before anything connects.
#[tracing::instrument(skip_all, fields(%name, lab_id = %lab_info.id))]
pub async fn console(name: &str, manifest: &Manifest, lab_info: &LabInfo) -> Result<()> {
    let node = console_node(name, manifest, lab_info)?;
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

/// The console of `name` on the Sherpa server: the node's address in the lab's loopback
/// network, from its place in the manifest, on the telnet port. A container has none.
fn console_node(name: &str, manifest: &Manifest, lab_info: &LabInfo) -> Result<Node> {
    let address = if name == BOOT_SERVER_NAME {
        get_ip(&lab_info.loopback_network, 255)
    } else {
        let (position, device) = manifest
            .nodes
            .iter()
            .enumerate()
            .find(|(_, device)| device.name == name)
            .ok_or_else(|| anyhow!("Device not found: {name}"))?;
        if device.model.kind() == NodeKind::Container {
            bail!(
                "{name} is a container, and containers have no serial console. Connect with \
                 'sherpa ssh {name}' instead."
            );
        }
        // Node indexes start from 1, as they do for addresses.
        let device_id = u8::try_from(position + 1)
            .with_context(|| format!("Device ID not found for: {name}"))?;
        get_ip(&lab_info.loopback_network, device_id)
    };
    Ok(Node::new(address.to_string(), TELNET_PORT))
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
        Error::TelnetTimeout { node, after, .. } => format!(
            "The console of {name} did not answer at {node} within {}s.",
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
    use std::io;
    use std::time::Duration;

    fn manifest() -> Manifest {
        toml::from_str(
            "name='test'\nnodes=[{name='alp01',model='alpine_linux'},{name='srl01',model='nokia_srlinux'}]\n",
        )
        .unwrap()
    }

    fn lab_info() -> LabInfo {
        "id='abcd1234'\nname='test'\nuser='alice'\nipv4_network='192.0.2.0/24'\nipv4_gateway='192.0.2.1'\nipv4_router='192.0.2.2'\nloopback_network='127.127.3.0/24'".parse().unwrap()
    }

    fn console() -> Node {
        Node::new("127.127.3.1", TELNET_PORT)
    }

    fn server() -> Node {
        Node::new("192.0.2.1", 22)
    }

    #[test]
    fn a_vm_console_is_on_its_loopback_address() {
        assert_eq!(
            console_node("alp01", &manifest(), &lab_info()).unwrap(),
            console()
        );
    }

    #[test]
    fn a_container_is_refused_before_anything_connects() {
        let error = console_node("srl01", &manifest(), &lab_info()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "srl01 is a container, and containers have no serial console. Connect with \
             'sherpa ssh srl01' instead."
        );
    }

    #[test]
    fn a_node_not_in_the_manifest_is_reported() {
        let error = console_node("nope01", &manifest(), &lab_info()).unwrap_err();
        assert_eq!(error.to_string(), "Device not found: nope01");
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
            "The console of alp01 did not answer at 127.127.3.1:2323 within 10s."
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
