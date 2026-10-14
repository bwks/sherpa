use std::ops::Deref;

use anyhow::Result;
use tracing::instrument;
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
    #[instrument(level = "debug", skip(self))]
    pub fn connect(&self) -> Result<QemuConnection> {
        // Suppress libvirt's default error handler which prints to stderr.
        // Errors are still returned via the Rust API.
        virt::error::clear_error_callback();

        let conn = Connect::open(Some(self.uri.as_str()))?;
        Ok(QemuConnection { conn })
    }
}

/// Wrapper around `virt::Connect` that closes the connection on drop.
///
/// The upstream `virt::Connect` does not implement `Drop`, so without
/// explicit cleanup every opened connection leaks a file descriptor.
pub struct QemuConnection {
    conn: Connect,
}

impl Deref for QemuConnection {
    type Target = Connect;

    fn deref(&self) -> &Connect {
        &self.conn
    }
}

impl Drop for QemuConnection {
    fn drop(&mut self) {
        if let Err(e) = self.conn.close() {
            tracing::error!("Failed to close libvirt connection: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_and_sync<T: Send + Sync>() {}

    /// `virt::Connect` is `Send + Sync`, so the wrapper is too without any `unsafe impl`.
    #[test]
    fn test_qemu_connection_is_send_and_sync() {
        assert_send_and_sync::<QemuConnection>();
    }
}
