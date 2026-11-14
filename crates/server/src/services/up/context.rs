//! What every phase of `up` works with.

use std::sync::Arc;

use bollard::Docker;

use crate::services::progress::ProgressSender;

use db::Database;
use libvirt::{Qemu, QemuConnection};
use shared::data::Config;

/// The lab being built, the server's connections, and where progress goes. Every phase takes
/// it; what one phase makes for the next is passed explicitly.
pub(super) struct LabBuild<'a> {
    pub(super) lab_id: &'a str,
    pub(super) lab_dir: &'a str,
    pub(super) db: &'a Database,
    pub(super) docker_conn: &'a Arc<Docker>,
    pub(super) qemu_conn: &'a Arc<QemuConnection>,
    /// Where `qemu_conn` came from: what the P2p link host opens its own connections with.
    pub(super) qemu: &'a Arc<Qemu>,
    pub(super) config: &'a Config,
    pub(super) progress: &'a ProgressSender,
}
