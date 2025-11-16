use std::fs;

use anyhow::Result;
use virt::storage_pool::StoragePool;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

use container::{docker_connection, kill_container, list_containers};
use konst::{CONTAINER_DNSMASQ_NAME, SHERPA_STORAGE_POOL, SHERPA_STORAGE_POOL_PATH, TEMP_DIR};
use libvirt::{Qemu, delete_disk};
use util::{dir_exists, file_exists, term_msg_surround};

pub async fn destroy(qemu: &Qemu, lab_name: &str, lab_id: &str) -> Result<()> {
    term_msg_surround(&format!("Destroying environment - {lab_name}-{lab_id}"));

    let qemu_conn = qemu.connect()?;
    let domains = qemu_conn.list_all_domains(0)?;
    let storage_pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;
    let pool_disks = storage_pool.list_volumes()?;

    let docker_conn = docker_connection()?;
    let lab_router = format!("{}-{}", CONTAINER_DNSMASQ_NAME, lab_id);
    for container in list_containers(&docker_conn).await? {
        if container
            // From docks: for historic reasons, names are prefixed with forward slash (/)
            .names
            .is_some_and(|x| x.contains(&format!("/{}", &lab_router)))
        {
            kill_container(&docker_conn, &lab_router).await?;
        }
    }

    for domain in domains {
        let vm_name = domain.get_name()?;
        if vm_name.contains(lab_id) && domain.is_active()? {
            // EUFI domains will have an NVRAM file that must be deleted.
            let nvram_flag = VIR_DOMAIN_UNDEFINE_NVRAM;
            domain.undefine_flags(nvram_flag)?;
            domain.destroy()?;
            println!("Destroyed VM: {vm_name}");

            // Destroy disks
            let domain_disks: Vec<&String> = pool_disks
                .iter()
                .filter(|d| d.starts_with(&vm_name))
                .collect();

            for disk in domain_disks {
                if file_exists(&format!("{SHERPA_STORAGE_POOL_PATH}/{disk}")) {
                    delete_disk(&qemu_conn, disk)?;
                    println!("Deleted disk: {disk}");
                }
            }
        }
    }

    let networks = qemu_conn.list_all_networks(0)?;
    for network in networks {
        if network.get_name()?.contains(lab_id) {
            let network_name = network.get_name()?;
            println!("Destroying network: {}", network_name);
            network.destroy()?;
            network.undefine()?;
            println!("Destroyed network: {}", network_name);
        }
    }

    if dir_exists(TEMP_DIR) {
        fs::remove_dir_all(TEMP_DIR)?;
        println!("Deleted directory: {TEMP_DIR}");
    }
    Ok(())
}
