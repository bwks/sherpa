use std::fs;

use anyhow::Result;
use virt::storage_pool::StoragePool;
use virt::sys::VIR_DOMAIN_UNDEFINE_NVRAM;

use container::{
    delete_network, docker_connection, kill_container, list_containers, list_networks,
};
use db::{connect, delete_lab, delete_lab_nodes};
use konst::{SHERPA_BASE_DIR, SHERPA_LABS_DIR, SHERPA_STORAGE_POOL, SHERPA_STORAGE_POOL_PATH};
use libvirt::{Qemu, delete_disk};
use network::{delete_interface, find_interfaces_fuzzy};
use util::{dir_exists, file_exists, term_msg_surround};

pub async fn destroy(qemu: &Qemu, lab_name: &str, lab_id: &str) -> Result<()> {
    term_msg_surround(&format!("Destroying environment - {lab_name}-{lab_id}"));
    let lab_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_LABS_DIR}/{lab_id}");

    let qemu_conn = qemu.connect()?;
    let domains = qemu_conn.list_all_domains(0)?;
    let storage_pool = StoragePool::lookup_by_name(&qemu_conn, SHERPA_STORAGE_POOL)?;
    let pool_disks = storage_pool.list_volumes()?;

    let docker_conn = docker_connection()?;
    for container in list_containers(&docker_conn).await? {
        if let Some(names) = &container.names {
            // Check if any container name contains the lab_id
            if names.iter().any(|name| name.contains(lab_id)) {
                // From docs: for historical reasons, container names start with a '/'
                // Extract the actual container name (remove leading /)
                if let Some(container_name) = names.first() {
                    let name = container_name.trim_start_matches('/');
                    kill_container(&docker_conn, name).await?;
                }
            }
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

    // Delete intefaces
    // testing
    let lab_intefaces = find_interfaces_fuzzy(lab_id).await?;
    for interface in lab_intefaces {
        // Only delete interfaces created outside of libvirt.
        //
        // Also, we will only delete the 'vea' end, the 'veb' end
        // will be automagically deleted when 'vea' is deleted.
        if interface.starts_with("vea")
            || interface.starts_with("bra")
            || interface.starts_with("brb")
        {
            delete_interface(&interface).await?;
            println!("Deleted interface: {}", interface);
        }
    }

    let container_networks = list_networks(&docker_conn).await?;
    for network in container_networks {
        if let Some(network_name) = network.name
            && network_name.contains(lab_id)
        {
            delete_network(&docker_conn, &network_name).await?;
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

    // Database
    let db = connect("localhost", 8000, "test", "test").await?;
    delete_lab_nodes(&db, lab_id).await?;
    delete_lab(&db, lab_id).await?;

    if dir_exists(&lab_dir) {
        fs::remove_dir_all(&lab_dir)?;
        println!("Deleted directory: {lab_dir}");
    }
    Ok(())
}
