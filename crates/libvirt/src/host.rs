use anyhow::{Context, Result};
use tracing::instrument;
use virt::connect::Connect;

/// The server's online CPUs and NUMA nodes, as libvirt reports them. Both are numbered from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostCpus {
    pub cpus: u32,
    pub numa_nodes: u32,
}

/// The server's CPUs and NUMA nodes.
#[instrument(level = "debug", skip(qemu_conn))]
pub fn host_cpus(qemu_conn: &Connect) -> Result<HostCpus> {
    let info = qemu_conn
        .get_node_info()
        .context("Failed to read the server's CPUs from libvirt")?;
    Ok(HostCpus {
        cpus: info.cpus,
        numa_nodes: info.nodes,
    })
}
