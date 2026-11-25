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
use tokio::task;
use tracing::instrument;

use libvirt::{
    ConsoleSockets, SerialConsole, VncConsole, move_consoles_to_sockets, serial_console,
    vnc_console,
};
use shared::data::{ModelKind, NodeKind};

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_owner;
use crate::daemon::state::AppState;
use crate::daemon::storage::restrict_dir;
use crate::konst::{CONSOLE_GROUP, CONSOLE_LAB_DIR_MODE, SHERPA_CONSOLE_DIR, UNIX_SOCKET_PATH_MAX};
use crate::services::error::{ServiceCategory, ServiceError};

/// A node console sherpad relays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConsoleKind {
    Serial,
    Vnc,
}

/// A node console the caller may open: the node and the domain it runs as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleTarget {
    pub lab_id: String,
    pub node_name: String,
    pub domain: String,
    pub microvm_serial: Option<String>,
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
        Err(error)
            if ServiceError::find(&error).and_then(ServiceError::category)
                == Some(ServiceCategory::Denied) =>
        {
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
    let in_microvm = image.model_kind().is_ok_and(ModelKind::runs_in_microvm);
    match (image.kind(), kind) {
        (NodeKind::Container, ConsoleKind::Serial) if in_microvm => {
            return Ok(ConsoleTarget {
                lab_id: lab_id.to_owned(),
                node_name: node_name.to_owned(),
                domain: format!("{node_name}-{lab_id}"),
                microvm_serial: Some(socket_path(lab_id, node.index, ConsoleKind::Serial)?),
            });
        }
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
        microvm_serial: None,
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
    if let Some(serial) = &target.microvm_serial {
        let reached = Path::new(serial)
            .exists()
            .then(|| Reached::Socket(serial.clone()));
        return socket_of(&target.node_name, kind, reached).map_err(Into::into);
    }
    let qemu = state.qemu.clone();
    let domain = target.domain.clone();
    let reached = task::spawn_blocking(move || -> Result<Option<Reached>> {
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
    Socket(String),
    Tcp,
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
    task::spawn_blocking(move || -> Result<()> {
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
    use nix::unistd::{self, Group};
    use shared::data::{ModelKind, NodeModel};

    #[test]
    fn console_sockets_006_destroy_removes_the_console_folder_and_a_folder_already_gone_is_no_error()
     {
        let root = tempfile::tempdir().unwrap();
        remove_console_dir(root.path(), "abcd1234").unwrap();
        let dir = console_dir(root.path(), "abcd1234");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("1.serial"), "").unwrap();
        remove_console_dir(root.path(), "abcd1234").unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn console_sockets_005_a_lab_console_folder_is_for_sherpa_and_kvm_only_mode_2770() {
        // The mode and group `create_console_dir` gives, as `restrict_dir` sets them.
        let root = tempfile::tempdir().unwrap();
        let dir = console_dir(root.path(), "abcd1234");
        let group = Group::from_gid(unistd::getgid()).unwrap().unwrap();
        restrict_dir(&dir, CONSOLE_LAB_DIR_MODE, Some(&group.name)).unwrap();
        let metadata = fs::metadata(&dir).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o2770);
        assert_eq!(metadata.gid(), group.gid.as_raw());
    }

    #[test]
    fn console_upgrade_008_a_running_node_still_on_tcp_is_refused_each_console() {
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
    }

    #[test]
    fn console_refusal_007_a_running_node_with_no_console_sherpa_can_reach_is_refused() {
        assert_eq!(
            socket_of("alp01", ConsoleKind::Vnc, Some(Reached::Unreachable)),
            Err(ServiceError::Conflict(
                "Node 'alp01' has no VNC console Sherpa can reach.".to_owned()
            ))
        );
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
                    microvm_serial: None,
                }
            );
            for (name, admin, expected) in [
                (
                    "bob",
                    false,
                    ServiceError::NotFound("Lab not found: abcd1234".to_owned()),
                ),
                (
                    "dana",
                    true,
                    ServiceError::Denied(
                        "only the owner of lab 'abcd1234' can open its consoles".to_owned(),
                    ),
                ),
            ] {
                let error = console_target(
                    &AuthContext::for_test(name, admin),
                    &db,
                    "abcd1234",
                    "node1",
                    kind,
                )
                .await
                .unwrap_err();
                assert_eq!(refusal(&error), Some(expected), "{name} {kind}");
            }
        }
    }

    #[tokio::test]
    async fn console_refusal_003_a_unikernels_vnc_console_is_refused_pointing_to_sherpa_console() {
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
