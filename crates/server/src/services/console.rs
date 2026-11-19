//! Node consoles, serial and VNC: UNIX sockets that only sherpad reaches, relayed to the lab's
//! owner.
//!
//! Each VM's and unikernel's serial console is a socket libvirt binds at
//! `<SHERPA_CONSOLE_DIR>/<lab-id>/<node-index>.serial`, and each VM's VNC console a socket QEMU
//! binds at `<node-index>.vnc` in the same folder. A lab's folder is `sherpa:kvm`, mode 2770, so
//! no other account on the server can reach a socket in it; it is made by `up`, `redeploy` and
//! `resume`, which start domains, and removed with the lab. sherpad relays a socket to the owner
//! over the API's console and VNC `WebSocket`s, one session per console (see
//! `api::handlers::console` and `docs/architecture/SERVER.md`).
//!
//! Domains defined before consoles moved behind sherpad have a telnet listener, a VNC port, or
//! both, on TCP instead. Their stored definitions are rewritten to the sockets when they are
//! started by `resume`, and at startup for those shut off ([`move_stopped_consoles`]); one still
//! running keeps its TCP listeners, and those consoles are refused, until it is stopped and
//! started again.

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use tracing::instrument;

use libvirt::{
    ConsoleSockets, SerialConsole, VncConsole, move_consoles_to_sockets, serial_console,
    vnc_console,
};
use shared::data::NodeKind;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_owner;
use crate::daemon::state::AppState;
use crate::daemon::storage::restrict_dir;
use crate::konst::{CONSOLE_GROUP, CONSOLE_LAB_DIR_MODE, SHERPA_CONSOLE_DIR, UNIX_SOCKET_PATH_MAX};
use crate::services::error::ServiceError;

/// A node console sherpad relays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConsoleKind {
    /// The serial console of a VM or unikernel.
    Serial,
    /// The VNC console of a VM.
    Vnc,
}

/// A node console the caller may open: the node and the domain it runs as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleTarget {
    pub lab_id: String,
    pub node_name: String,
    /// The node's libvirt domain, `<node>-<lab-id>`.
    pub domain: String,
}

/// The folder of lab `lab_id`'s console sockets under `root`.
#[must_use]
pub fn console_dir(root: &Path, lab_id: &str) -> PathBuf {
    root.join(lab_id)
}

/// The console sockets of the node with index `node_index` in lab `lab_id`.
///
/// # Errors
///
/// Fails when a path is longer than a UNIX socket path can be, as libvirt would refuse to start
/// the domain.
pub fn console_sockets(lab_id: &str, node_index: u16) -> Result<ConsoleSockets> {
    Ok(ConsoleSockets {
        serial: socket_path(lab_id, node_index, ConsoleKind::Serial)?,
        vnc: socket_path(lab_id, node_index, ConsoleKind::Vnc)?,
    })
}

/// The `kind` console socket of the node with index `node_index` in lab `lab_id`.
///
/// # Errors
///
/// Fails when the path is longer than a UNIX socket path can be.
fn socket_path(lab_id: &str, node_index: u16, kind: ConsoleKind) -> Result<String> {
    let path = format!(
        "{SHERPA_CONSOLE_DIR}/{lab_id}/{node_index}.{}",
        kind.suffix()
    );
    if path.len() > UNIX_SOCKET_PATH_MAX {
        bail!(
            "The {kind} socket of node {node_index} in lab '{lab_id}' would be {path}, {} bytes, \
             longer than the {UNIX_SOCKET_PATH_MAX} a UNIX socket path can be. Use a shorter lab \
             ID.",
            path.len()
        );
    }
    Ok(path)
}

/// Create lab `lab_id`'s console folder, `sherpa:kvm` mode 2770, if it is missing, and set its
/// owner and mode if it is not. Called before a domain of the lab starts, as libvirt cannot bind
/// a socket in a folder that does not exist.
///
/// # Errors
///
/// Fails when the folder cannot be created, given to the `kvm` group or set to its mode.
#[instrument(level = "debug")]
pub fn create_console_dir(lab_id: &str) -> Result<()> {
    restrict_dir(
        &console_dir(Path::new(SHERPA_CONSOLE_DIR), lab_id),
        CONSOLE_LAB_DIR_MODE,
        Some(CONSOLE_GROUP),
    )
    .with_context(|| format!("Cannot create the console folder of lab '{lab_id}'"))
}

/// Remove lab `lab_id`'s console folder under `root`, and any socket left in it. A folder that
/// is not there is not an error.
///
/// # Errors
///
/// Fails when the folder exists and cannot be removed.
#[instrument(level = "debug")]
pub fn remove_console_dir(root: &Path, lab_id: &str) -> Result<()> {
    let dir = console_dir(root, lab_id);
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("Cannot remove the console folder {}", dir.display())),
    }
}

/// The `kind` console of node `node_name` in lab `lab_id`, when `ctx` may open it: only the
/// lab's owner may, admins included; only a VM or a unikernel has a serial console, and only a
/// VM a VNC console.
///
/// # Errors
///
/// - [`ServiceError::NotFound`] when the lab or the node does not exist.
/// - [`ServiceError::Denied`] when the caller does not own the lab.
/// - [`ServiceError::Invalid`] when the node has no such console: a container, or a unikernel's
///   VNC.
/// - Any other error when the database cannot be read.
#[instrument(skip(ctx, db), fields(username = %ctx.username, %lab_id, %node_name, %kind))]
pub async fn console_target(
    ctx: &AuthContext,
    db: &db::Database,
    lab_id: &str,
    node_name: &str,
    kind: ConsoleKind,
) -> Result<ConsoleTarget> {
    let lab = match require_lab_owner(ctx, db, lab_id).await {
        Ok(lab) => lab,
        Err(error) if matches!(ServiceError::find(&error), Some(ServiceError::Denied(_))) => {
            return Err(ServiceError::Denied(format!(
                "only the owner of lab '{lab_id}' can open its consoles"
            ))
            .into());
        }
        Err(error) => return Err(error),
    };
    let lab_record = lab
        .id
        .ok_or_else(|| anyhow!("Lab '{lab_id}' has no record ID"))?;
    let node = db::list_nodes_by_lab(db, lab_record)
        .await
        .context("Failed to read the lab's nodes")?
        .into_iter()
        .find(|node| node.name == node_name)
        .ok_or_else(|| {
            ServiceError::NotFound(format!("Node '{node_name}' not found in lab '{lab_id}'"))
        })?;
    let image = db::find_node_image_by_id(db, node.image.clone())
        .await
        .context("Failed to read the node's image")?
        .ok_or_else(|| anyhow!("Node '{node_name}' has no image"))?;
    match (image.kind(), kind) {
        (NodeKind::Container, _) => {
            return Err(ServiceError::Invalid(format!(
                "{node_name} is a container, and containers have no {kind}. Connect with \
                 'sherpa ssh {node_name}' instead."
            ))
            .into());
        }
        (NodeKind::Unikernel, ConsoleKind::Vnc) => {
            return Err(ServiceError::Invalid(format!(
                "{node_name} is a unikernel, and unikernels have no VNC console. Use 'sherpa \
                 console {node_name}' instead."
            ))
            .into());
        }
        (NodeKind::VirtualMachine | NodeKind::Unikernel, _) => {}
    }
    Ok(ConsoleTarget {
        lab_id: lab_id.to_owned(),
        node_name: node_name.to_owned(),
        domain: format!("{node_name}-{lab_id}"),
    })
}

/// The socket of `target`'s `kind` console, as its running domain was started with it.
///
/// # Errors
///
/// - [`ServiceError::Conflict`] when the node is not running, was started before the console
///   moved behind sherpad and still listens on TCP, or has no such socket.
/// - Any other error when libvirt cannot be reached or the domain read.
#[instrument(skip(state), fields(lab_id = %target.lab_id, node_name = %target.node_name, %kind))]
pub async fn console_socket(
    state: &AppState,
    target: &ConsoleTarget,
    kind: ConsoleKind,
) -> Result<String> {
    let qemu = state.qemu.clone();
    let domain = target.domain.clone();
    let reached = tokio::task::spawn_blocking(move || -> Result<Option<Reached>> {
        let conn = qemu.connect().context("Failed to connect to libvirt")?;
        Ok(match kind {
            ConsoleKind::Serial => serial_console(&conn, &domain)?.map(Reached::from),
            ConsoleKind::Vnc => vnc_console(&conn, &domain)?.map(Reached::from),
        })
    })
    .await
    .context("The libvirt task failed")??;
    socket_of(&target.node_name, kind, reached).map_err(Into::into)
}

/// How a running domain's console is reached, serial or VNC alike.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reached {
    /// A UNIX socket at this path.
    Socket(String),
    /// A TCP listener, from before the console moved behind sherpad.
    Tcp,
    /// Nothing Sherpa can reach.
    Unreachable,
}

/// The socket path of node `node_name`'s `kind` console reached as `reached`, `None` when the
/// node is not running, or why it cannot be opened.
fn socket_of(
    node_name: &str,
    kind: ConsoleKind,
    reached: Option<Reached>,
) -> Result<String, ServiceError> {
    match reached {
        Some(Reached::Socket(path)) => Ok(path),
        None => Err(ServiceError::Conflict(format!(
            "Node '{node_name}' is not running. Start it with 'sherpa resume'."
        ))),
        Some(Reached::Tcp) => Err(ServiceError::Conflict(match kind {
            ConsoleKind::Serial => format!(
                "Node '{node_name}' was started before consoles moved behind Sherpa. Stop and \
                 resume the lab ('sherpa down', then 'sherpa resume'), or redeploy the node, to \
                 use its console."
            ),
            ConsoleKind::Vnc => format!(
                "Node '{node_name}' was started before its VNC console moved behind Sherpa. Stop \
                 and resume the lab ('sherpa down', then 'sherpa resume'), or redeploy the node, \
                 to use its VNC console."
            ),
        })),
        Some(Reached::Unreachable) => Err(ServiceError::Conflict(format!(
            "Node '{node_name}' has no {kind} Sherpa can reach."
        ))),
    }
}

impl From<SerialConsole> for Reached {
    fn from(console: SerialConsole) -> Self {
        match console {
            SerialConsole::Socket(path) => Self::Socket(path),
            SerialConsole::Tcp => Self::Tcp,
            SerialConsole::Other(_) | SerialConsole::Absent => Self::Unreachable,
        }
    }
}

impl From<VncConsole> for Reached {
    fn from(console: VncConsole) -> Self {
        match console {
            VncConsole::Socket(path) => Self::Socket(path),
            VncConsole::Tcp => Self::Tcp,
            VncConsole::Absent => Self::Unreachable,
        }
    }
}

impl ConsoleKind {
    /// The file name suffix of the console's socket.
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Serial => "serial",
            Self::Vnc => "vnc",
        }
    }

    /// The console's short name in a message: `console`, as the serial console has always been
    /// called, or `VNC console`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Serial => "console",
            Self::Vnc => "VNC console",
        }
    }

    /// The `sherpa` command that opens the console.
    #[must_use]
    pub fn command(self) -> &'static str {
        match self {
            Self::Serial => "console",
            Self::Vnc => "vnc",
        }
    }
}

impl fmt::Display for ConsoleKind {
    /// The console's name in a sentence: `serial console` or `VNC console`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serial => write!(f, "serial console"),
            Self::Vnc => write!(f, "VNC console"),
        }
    }
}

/// Before the server takes requests: give every lab its console folder, and move the serial and
/// VNC consoles of each shut-off VM and unikernel still on TCP to their sockets, so it comes up
/// there at its next start, by `resume` or otherwise. Running ones with a console on TCP are
/// counted in a warning: those consoles are refused until they stop. A lab or node that fails is logged and the rest go on.
///
/// # Errors
///
/// Fails when the labs cannot be read from the database or libvirt cannot be reached.
#[instrument(skip(state))]
pub async fn move_stopped_consoles(state: &AppState) -> Result<()> {
    let mut domains = Vec::new();
    for lab in db::list_labs(&state.db)
        .await
        .context("Failed to read the labs")?
    {
        if let Err(error) = create_console_dir(&lab.lab_id) {
            tracing::warn!(lab_id = %lab.lab_id, error = %format!("{error:#}"), "Cannot create a lab's console folder");
        }
        let Some(record) = lab.id else {
            continue;
        };
        for node in db::list_nodes_by_lab(&state.db, record)
            .await
            .with_context(|| format!("Failed to read the nodes of lab '{}'", lab.lab_id))?
        {
            match console_sockets(&lab.lab_id, node.index) {
                Ok(sockets) => domains.push((format!("{}-{}", node.name, lab.lab_id), sockets)),
                Err(error) => {
                    tracing::warn!(lab_id = %lab.lab_id, node = %node.name, error = %format!("{error:#}"), "Cannot place a node's console socket");
                }
            }
        }
    }
    let qemu = state.qemu.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = qemu.connect().context("Failed to connect to libvirt")?;
        let mut moved = 0usize;
        let mut running_on_tcp = 0usize;
        for (domain, sockets) in domains {
            // A container, or a node with no domain, has none to move.
            let running = serial_console(&conn, &domain)
                .and_then(|serial| Ok((serial, vnc_console(&conn, &domain)?)));
            let running = match running {
                Ok(consoles) => consoles,
                Err(error) => {
                    tracing::warn!(%domain, error = %format!("{error:#}"), "Cannot read a domain's consoles");
                    continue;
                }
            };
            match running {
                (Some(SerialConsole::Tcp), _) | (_, Some(VncConsole::Tcp)) => running_on_tcp += 1,
                (Some(_), _) | (_, Some(_)) => {}
                (None, None) => match move_consoles_to_sockets(&conn, &domain, &sockets) {
                    Ok(true) => moved += 1,
                    Ok(false) => {}
                    Err(error) => tracing::debug!(%domain, error = %format!("{error:#}"), "No console moved"),
                },
            }
        }
        if moved > 0 {
            tracing::info!(domains = moved, "Moved the consoles of stopped nodes to their sockets");
        }
        if running_on_tcp > 0 {
            tracing::warn!(
                nodes = running_on_tcp,
                "Running nodes were started before consoles moved behind sherpad: their serial or \
                 VNC consoles listen on TCP on the server's loopback and are refused through \
                 Sherpa until each is stopped and resumed, or redeployed"
            );
        }
        Ok(())
    })
    .await
    .context("The libvirt task failed")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use crate::services::node_lifecycle::tests::lab_of;
    use db::{DefaultRequest, upsert_node_image};
    use shared::data::{ModelKind, NodeModel};

    #[test]
    fn console_sockets_are_named_by_their_lab_and_node_index() {
        assert_eq!(
            console_sockets("abcd1234", 7).unwrap(),
            ConsoleSockets {
                serial: "/opt/sherpa/run/console/abcd1234/7.serial".to_owned(),
                vnc: "/opt/sherpa/run/console/abcd1234/7.vnc".to_owned(),
            }
        );
    }

    #[test]
    fn a_socket_path_past_the_unix_limit_is_refused() {
        // The folder, two slashes, the index and the longer suffix leave the rest of 107 bytes
        // to the lab ID.
        let fixed = SHERPA_CONSOLE_DIR.len() + "//1.serial".len();
        let longest = "a".repeat(UNIX_SOCKET_PATH_MAX - fixed);
        assert_eq!(
            console_sockets(&longest, 1).unwrap().serial.len(),
            UNIX_SOCKET_PATH_MAX
        );
        let too_long = format!("{longest}b");
        let error = console_sockets(&too_long, 1).unwrap_err().to_string();
        assert!(error.contains("The serial console socket"), "{error}");
        assert!(error.contains("108 bytes"), "{error}");
        assert!(error.contains("longer than the 107"), "{error}");
        // The VNC socket's shorter suffix leaves room for a longer lab ID, but not past 107.
        let vnc_longest = "a".repeat(UNIX_SOCKET_PATH_MAX - fixed + 3);
        let error = socket_path(&format!("{vnc_longest}b"), 1, ConsoleKind::Vnc)
            .unwrap_err()
            .to_string();
        assert!(error.contains("The VNC console socket"), "{error}");
        assert_eq!(
            socket_path(&vnc_longest, 1, ConsoleKind::Vnc)
                .unwrap()
                .len(),
            UNIX_SOCKET_PATH_MAX
        );
    }

    #[test]
    fn a_console_folder_that_is_not_there_is_already_removed() {
        let root = tempfile::tempdir().unwrap();
        remove_console_dir(root.path(), "abcd1234").unwrap();
        let dir = console_dir(root.path(), "abcd1234");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("1.serial"), "").unwrap();
        remove_console_dir(root.path(), "abcd1234").unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn a_console_folder_is_reachable_only_by_its_owner_and_group() {
        // The mode and group `create_console_dir` gives, as `restrict_dir` sets them.
        let root = tempfile::tempdir().unwrap();
        let dir = console_dir(root.path(), "abcd1234");
        let group = nix::unistd::Group::from_gid(nix::unistd::getgid())
            .unwrap()
            .unwrap();
        restrict_dir(&dir, CONSOLE_LAB_DIR_MODE, Some(&group.name)).unwrap();
        let metadata = fs::metadata(&dir).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o2770);
        assert_eq!(metadata.gid(), group.gid.as_raw());
    }

    #[test]
    fn a_running_socket_console_is_opened_and_anything_else_is_refused() {
        for kind in [ConsoleKind::Serial, ConsoleKind::Vnc] {
            assert_eq!(
                socket_of("alp01", kind, Some(Reached::Socket("/s".to_owned()))),
                Ok("/s".to_owned())
            );
            assert_eq!(
                socket_of("alp01", kind, None),
                Err(ServiceError::Conflict(
                    "Node 'alp01' is not running. Start it with 'sherpa resume'.".to_owned()
                ))
            );
        }
        assert_eq!(
            socket_of("alp01", ConsoleKind::Serial, Some(Reached::Tcp)),
            Err(ServiceError::Conflict(
                "Node 'alp01' was started before consoles moved behind Sherpa. Stop and resume \
                 the lab ('sherpa down', then 'sherpa resume'), or redeploy the node, to use its \
                 console."
                    .to_owned()
            ))
        );
        assert_eq!(
            socket_of("alp01", ConsoleKind::Vnc, Some(Reached::Tcp)),
            Err(ServiceError::Conflict(
                "Node 'alp01' was started before its VNC console moved behind Sherpa. Stop and \
                 resume the lab ('sherpa down', then 'sherpa resume'), or redeploy the node, to \
                 use its VNC console."
                    .to_owned()
            ))
        );
        assert_eq!(
            socket_of("alp01", ConsoleKind::Vnc, Some(Reached::Unreachable)),
            Err(ServiceError::Conflict(
                "Node 'alp01' has no VNC console Sherpa can reach.".to_owned()
            ))
        );
    }

    #[test]
    fn a_console_is_reached_as_libvirt_reports_it() {
        assert_eq!(
            Reached::from(SerialConsole::Socket("/s".to_owned())),
            Reached::Socket("/s".to_owned())
        );
        assert_eq!(Reached::from(SerialConsole::Tcp), Reached::Tcp);
        assert_eq!(
            Reached::from(SerialConsole::Other("pty".to_owned())),
            Reached::Unreachable
        );
        assert_eq!(Reached::from(SerialConsole::Absent), Reached::Unreachable);
        assert_eq!(
            Reached::from(VncConsole::Socket("/v".to_owned())),
            Reached::Socket("/v".to_owned())
        );
        assert_eq!(Reached::from(VncConsole::Tcp), Reached::Tcp);
        assert_eq!(Reached::from(VncConsole::Absent), Reached::Unreachable);
    }

    fn refusal(error: &anyhow::Error) -> Option<ServiceError> {
        ServiceError::find(error).cloned()
    }

    #[tokio::test]
    async fn only_the_lab_owner_may_open_a_console() {
        let (db, _, _) = lab_of("abcd1234", 1).await;
        db::create_user(&db, "bob".to_owned(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        db::create_user(&db, "dana".to_owned(), "TestPass123!", true, vec![])
            .await
            .unwrap();
        let owner = AuthContext::for_test("alice", false);
        for kind in [ConsoleKind::Serial, ConsoleKind::Vnc] {
            assert_eq!(
                console_target(&owner, &db, "abcd1234", "node1", kind)
                    .await
                    .unwrap(),
                ConsoleTarget {
                    lab_id: "abcd1234".to_owned(),
                    node_name: "node1".to_owned(),
                    domain: "node1-abcd1234".to_owned(),
                }
            );
            for (name, admin) in [("bob", false), ("dana", true)] {
                let error = console_target(
                    &AuthContext::for_test(name, admin),
                    &db,
                    "abcd1234",
                    "node1",
                    kind,
                )
                .await
                .unwrap_err();
                assert_eq!(
                    refusal(&error),
                    Some(ServiceError::Denied(
                        "only the owner of lab 'abcd1234' can open its consoles".to_owned()
                    )),
                    "{name} {kind}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_missing_lab_or_node_is_not_found() {
        let (db, _, _) = lab_of("abcd1234", 1).await;
        let owner = AuthContext::for_test("alice", false);
        let error = console_target(&owner, &db, "nope0000", "node1", ConsoleKind::Vnc)
            .await
            .unwrap_err();
        assert!(matches!(refusal(&error), Some(ServiceError::NotFound(_))));
        let error = console_target(&owner, &db, "abcd1234", "node9", ConsoleKind::Serial)
            .await
            .unwrap_err();
        assert_eq!(
            refusal(&error),
            Some(ServiceError::NotFound(
                "Node 'node9' not found in lab 'abcd1234'".to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn a_container_has_no_console() {
        let (db, lab, _) = lab_of("abcd1234", 0).await;
        let image = upsert_node_image(
            &db,
            ModelKind::default_for(NodeModel::NokiaSrlinux).node_config(),
            DefaultRequest::Keep,
        )
        .await
        .unwrap();
        db::create_node(&db, "srl01", 1, image.id.unwrap(), lab)
            .await
            .unwrap();
        let owner = AuthContext::for_test("alice", false);
        for (kind, message) in [
            (
                ConsoleKind::Serial,
                "srl01 is a container, and containers have no serial console. Connect with \
                 'sherpa ssh srl01' instead.",
            ),
            (
                ConsoleKind::Vnc,
                "srl01 is a container, and containers have no VNC console. Connect with 'sherpa \
                 ssh srl01' instead.",
            ),
        ] {
            let error = console_target(&owner, &db, "abcd1234", "srl01", kind)
                .await
                .unwrap_err();
            assert_eq!(
                refusal(&error),
                Some(ServiceError::Invalid(message.to_owned()))
            );
        }
    }

    #[tokio::test]
    async fn a_unikernel_has_a_serial_console_and_no_vnc_console() {
        let (db, lab, _) = lab_of("abcd1234", 0).await;
        let image = upsert_node_image(
            &db,
            ModelKind::new(NodeModel::Generic, Some(NodeKind::Unikernel))
                .unwrap()
                .node_config(),
            DefaultRequest::Keep,
        )
        .await
        .unwrap();
        db::create_node(&db, "uk01", 1, image.id.unwrap(), lab)
            .await
            .unwrap();
        let owner = AuthContext::for_test("alice", false);
        console_target(&owner, &db, "abcd1234", "uk01", ConsoleKind::Serial)
            .await
            .unwrap();
        let error = console_target(&owner, &db, "abcd1234", "uk01", ConsoleKind::Vnc)
            .await
            .unwrap_err();
        assert_eq!(
            refusal(&error),
            Some(ServiceError::Invalid(
                "uk01 is a unikernel, and unikernels have no VNC console. Use 'sherpa console \
                 uk01' instead."
                    .to_owned()
            ))
        );
    }
}
