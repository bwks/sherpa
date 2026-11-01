use std::fmt;

use serde_derive::{Deserialize, Serialize};

use super::CpuModels;

/// Performance tuning for a virtual machine node: `[nodes.performance_tuning]` in the manifest.
/// Every setting is optional; one left out keeps the domain as it is without tuning.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PerformanceTuning {
    pub cpu_mode: Option<CpuMode>,
    pub cpu_topology: Option<CpuTopology>,
    /// The host CPU for each vCPU, in vCPU order.
    pub cpu_pinning: Option<Vec<u16>>,
    /// Host CPUs for QEMU's own threads: everything in the QEMU process except the vCPUs.
    pub qemu_cpus: Option<Vec<u16>>,
    /// The host NUMA node the guest's memory is held on.
    pub numa_node: Option<u8>,
    pub disk_cache: Option<DiskCache>,
    pub disk_io: Option<DiskIo>,
    /// Gives the disks an I/O thread, pinned to these host CPUs.
    pub disk_io_thread_cpus: Option<Vec<u16>>,
    /// virtio-net queues on each virtio interface.
    pub interface_queues: Option<u8>,
}

/// The host CPUs one node is pinned to, for checking that nodes do not overlap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeCpuPins {
    pub node_name: String,
    /// Host CPUs running the node's vCPUs, which no other thread may share.
    pub vcpu_cpus: Vec<u16>,
    /// Host CPUs running the node's QEMU and I/O threads.
    pub qemu_thread_cpus: Vec<u16>,
}

/// How the guest CPU is presented.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CpuMode {
    /// The closest named model to the host CPU, with its features.
    HostModel,
    /// The host CPU exactly.
    HostPassthrough,
}

/// The CPU layout the guest sees.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CpuTopology {
    pub sockets: u8,
    pub cores: u8,
    pub threads: u8,
}

/// The host cache mode for a node's disks.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiskCache {
    None,
    Writeback,
    Writethrough,
}

/// How QEMU submits disk I/O.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiskIo {
    Native,
    IoUring,
    Threads,
}

impl From<CpuMode> for CpuModels {
    fn from(mode: CpuMode) -> Self {
        match mode {
            CpuMode::HostModel => CpuModels::HostModel,
            CpuMode::HostPassthrough => CpuModels::HostPassthrough,
        }
    }
}

impl fmt::Display for DiskCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskCache::None => write!(f, "none"),
            DiskCache::Writeback => write!(f, "writeback"),
            DiskCache::Writethrough => write!(f, "writethrough"),
        }
    }
}

impl fmt::Display for DiskIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskIo::Native => write!(f, "native"),
            DiskIo::IoUring => write!(f, "io_uring"),
            DiskIo::Threads => write!(f, "threads"),
        }
    }
}

impl PerformanceTuning {
    /// The host CPUs running QEMU's threads other than vCPUs: its own and the disk I/O thread.
    pub fn qemu_thread_cpus(&self) -> Vec<u16> {
        let mut cpus: Vec<u16> = self
            .qemu_cpus
            .iter()
            .chain(self.disk_io_thread_cpus.iter())
            .flatten()
            .copied()
            .collect();
        cpus.sort_unstable();
        cpus.dedup();
        cpus
    }
}

impl NodeCpuPins {
    /// The pins of a node with this tuning.
    pub fn of(node_name: &str, tuning: &PerformanceTuning) -> Self {
        Self {
            node_name: node_name.to_string(),
            vcpu_cpus: tuning.cpu_pinning.clone().unwrap_or_default(),
            qemu_thread_cpus: tuning.qemu_thread_cpus(),
        }
    }
}

/// A libvirt cpuset for host CPUs, such as `8,24`.
pub fn cpuset(cpus: &[u16]) -> String {
    cpus.iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_tuning_reads_every_setting() {
        let tuning: PerformanceTuning = serde_json::from_value(serde_json::json!({
            "cpu_mode": "host-passthrough",
            "cpu_topology": { "sockets": 1, "cores": 7, "threads": 2 },
            "cpu_pinning": [9, 25],
            "qemu_cpus": [8, 24],
            "numa_node": 1,
            "disk_cache": "none",
            "disk_io": "io_uring",
            "disk_io_thread_cpus": [8, 24],
            "interface_queues": 4
        }))
        .unwrap();
        assert_eq!(tuning.cpu_mode, Some(CpuMode::HostPassthrough));
        assert_eq!(
            tuning.cpu_topology,
            Some(CpuTopology {
                sockets: 1,
                cores: 7,
                threads: 2
            })
        );
        assert_eq!(tuning.disk_cache, Some(DiskCache::None));
        assert_eq!(tuning.disk_io, Some(DiskIo::IoUring));
        assert_eq!(tuning.interface_queues, Some(4));
    }

    #[test]
    fn test_performance_tuning_rejects_unknown_settings() {
        let result: Result<PerformanceTuning, _> =
            serde_json::from_value(serde_json::json!({ "cpu_pinning_typo": [1] }));
        assert!(result.is_err());
    }

    #[test]
    fn test_cpu_mode_maps_to_cpu_model() {
        assert_eq!(
            CpuModels::from(CpuMode::HostPassthrough),
            CpuModels::HostPassthrough
        );
        assert_eq!(CpuModels::from(CpuMode::HostModel), CpuModels::HostModel);
    }

    #[test]
    fn test_disk_settings_display_as_libvirt_values() {
        assert_eq!(DiskCache::None.to_string(), "none");
        assert_eq!(DiskCache::Writethrough.to_string(), "writethrough");
        assert_eq!(DiskIo::IoUring.to_string(), "io_uring");
        assert_eq!(DiskIo::Native.to_string(), "native");
    }

    #[test]
    fn test_qemu_thread_cpus_joins_qemu_and_io_thread_cpus() {
        let tuning = PerformanceTuning {
            qemu_cpus: Some(vec![24, 8]),
            disk_io_thread_cpus: Some(vec![8, 7]),
            ..PerformanceTuning::default()
        };
        assert_eq!(tuning.qemu_thread_cpus(), vec![7, 8, 24]);
        assert!(PerformanceTuning::default().qemu_thread_cpus().is_empty());
    }

    #[test]
    fn test_cpuset_lists_cpus() {
        assert_eq!(cpuset(&[8, 24]), "8,24");
        assert_eq!(cpuset(&[3]), "3");
    }
}
