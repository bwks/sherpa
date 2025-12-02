use anyhow::{Context, Result};
use virt::connect::Connect;
use virt::storage_pool::StoragePool;

pub struct SherpaStoragePool {
    pub name: String,
    pub path: String,
}
impl SherpaStoragePool {
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
                println!("Storage pool '{name}' already exists");
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

                println!("Storage pool '{name}' created and started");
                Ok(())
            }
        }
    }
}
