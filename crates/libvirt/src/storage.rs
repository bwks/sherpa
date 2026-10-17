use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use shared::konst::{STORAGE_POOL_REFRESH_ATTEMPTS, STORAGE_POOL_REFRESH_RETRY_MS};
use tracing::instrument;
use virt::connect::Connect;
use virt::storage_pool::StoragePool;

pub struct SherpaStoragePool {
    pub name: String,
    pub path: String,
}
impl SherpaStoragePool {
    #[instrument(level = "debug", skip(self, qemu_conn))]
    pub fn create(self, qemu_conn: &Connect) -> Result<()> {
        let name = self.name;
        let path = self.path;

        // Define th<-e storage pool XML
        let pool_xml = format!(
            r#"<pool type='dir'>
            <name>{name}</name>
            <target>
                <path>{path}</path>
            </target>
        </pool>"#
        );

        // Check if the pool already exists
        match StoragePool::lookup_by_name(qemu_conn, &name) {
            Ok(_pool) => {
                tracing::debug!(pool_name = %name, "Storage pool already exists");
                Ok(())
            }
            Err(_) => {
                // Define the storage pool
                let pool = StoragePool::define_xml(qemu_conn, &pool_xml, 0)
                    .context("Failed to define storage pool")?;

                // Build the storage pool (this will create the directory)
                pool.build(0).context("Failed to build storage pool")?;

                // Start the storage pool
                pool.create(0).context("Failed to start storage pool")?;

                // Set the pool to autostart
                pool.set_autostart(true)
                    .context("Failed to set pool autostart")?;

                tracing::info!(pool_name = %name, path = %path, "Storage pool created and started");
                Ok(())
            }
        }
    }
}

/// Refresh a storage pool, retrying while another operation holds it.
///
/// libvirt refuses a refresh while asynchronous jobs, such as a volume upload for
/// another node, run on the pool. It reports this only as a generic internal error, so
/// every refresh error is retried for a bounded time and the last one is returned.
#[instrument(level = "debug", skip(pool))]
pub fn refresh_pool(pool: &StoragePool) -> Result<()> {
    let mut attempt = 1;
    loop {
        match pool.refresh(0) {
            Ok(_) => return Ok(()),
            Err(error) if attempt < STORAGE_POOL_REFRESH_ATTEMPTS => {
                tracing::debug!(attempt, error = %error, "Storage pool busy; retrying refresh");
                thread::sleep(Duration::from_millis(STORAGE_POOL_REFRESH_RETRY_MS));
                attempt += 1;
            }
            Err(error) => {
                return Err(error).context(format!(
                    "Storage pool refresh still failing after {STORAGE_POOL_REFRESH_ATTEMPTS} attempts"
                ));
            }
        }
    }
}
