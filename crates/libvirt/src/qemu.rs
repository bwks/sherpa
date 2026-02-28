use std::ops::Deref;

use anyhow::Result;
use virt::connect::Connect;

use shared::konst::QEMU_URI;

pub struct Qemu {
    pub uri: String,
}

impl Default for Qemu {
    fn default() -> Self {
        Self {
            uri: QEMU_URI.to_owned(),
        }
    }
}

impl Qemu {
    pub fn connect(&self) -> Result<QemuConnection> {
        let conn = Connect::open(Some(self.uri.as_str()))?;
        Ok(QemuConnection { conn: Some(conn) })
    }
}

/// Wrapper around `virt::Connect` that closes the connection on drop.
///
/// The upstream `virt::Connect` does not implement `Drop`, so without
/// explicit cleanup every opened connection leaks a file descriptor.
pub struct QemuConnection {
    conn: Option<Connect>,
}

unsafe impl Send for QemuConnection {}
unsafe impl Sync for QemuConnection {}

impl Deref for QemuConnection {
    type Target = Connect;

    fn deref(&self) -> &Connect {
        self.conn.as_ref().expect("QemuConnection used after drop")
    }
}

impl Drop for QemuConnection {
    fn drop(&mut self) {
        if let Some(mut conn) = self.conn.take() {
            if let Err(e) = conn.close() {
                tracing::error!("Failed to close libvirt connection: {}", e);
            }
        }
    }
}
