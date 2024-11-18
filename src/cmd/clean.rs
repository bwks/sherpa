use anyhow::Result;

use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;

use crate::core::konst::SHERPA_STORAGE_POOL;
use crate::libvirt::Qemu;
use crate::util::{get_id, term_msg_surround};

pub fn clean(qemu: &Qemu, all: bool, disks: bool, networks: bool) -> Result<()> {
    if all {
        // term_msg_surround("Cleaning environment");
        term_msg_surround("Not implemented");
    } else if disks {
        term_msg_surround("Cleaning disks");
        let lab_id = get_id()?;

        let qemu_conn = qemu.connect()?;

        let pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;
        for volume in pool.list_volumes()? {
            if volume.contains(&lab_id) {
                println!("Deleting disk: {}", volume);
                let vol = StorageVol::lookup_by_name(&pool, &volume)?;
                vol.delete(0)?;
                println!("Deleted disk: {}", volume);
            }
        }
    } else if networks {
        term_msg_surround("Cleaning networks");

        let qemu_conn = qemu.connect()?;

        let networks = qemu_conn.list_all_networks(0)?;
        for network in networks {
            if network.get_name()?.contains("sherpa") {
                let network_name = network.get_name()?;
                println!("Destroying network: {}", network_name);
                network.destroy()?;
                network.undefine()?;
                println!("Destroyed network: {}", network_name);
            }
        }
    }
    Ok(())
}
