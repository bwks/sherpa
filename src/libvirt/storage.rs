use anyhow::{Context, Result};
use std::fs;
use virt::connect::Connect;
use virt::storage_pool::StoragePool;

use crate::core::konst::{SHERPA_STORAGE_POOL, SHERPA_STORAGE_POOL_PATH};

pub fn create_sherpa_storage_pool(conn: &Connect) -> Result<StoragePool> {
    const SHERPA_DIR: &str = SHERPA_STORAGE_POOL_PATH;
    const POOL_NAME: &str = SHERPA_STORAGE_POOL;

    // Define the storage pool XML
    let pool_xml = format!(
        r#"<pool type='dir'>
            <name>{}</name>
            <target>
                <path>{}</path>
            </target>
        </pool>"#,
        POOL_NAME, SHERPA_DIR
    );

    // Check if the pool already exists
    match StoragePool::lookup_by_name(conn, POOL_NAME) {
        Ok(pool) => {
            println!("Storage pool '{}' already exists", POOL_NAME);
            Ok(pool)
        }
        Err(_) => {
            // Define the storage pool
            let pool = StoragePool::define_xml(conn, &pool_xml, 0)
                .context("Failed to define storage pool")?;

            // Build the storage pool (this will create the directory)
            pool.build(0).context("Failed to build storage pool")?;

            // Start the storage pool
            pool.create(0).context("Failed to start storage pool")?;

            // Set the pool to autostart
            pool.set_autostart(true)
                .context("Failed to set pool autostart")?;

            println!("Storage pool '{}' created and started", POOL_NAME);
            Ok(pool)
        }
    }
}
