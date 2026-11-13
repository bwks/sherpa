use std::fmt;

use serde_derive::{Deserialize, Serialize};

use super::{CpuModels, DiskImageCloneMethod};

/// `[nodes.cpu]` in the manifest: the node's vCPU count, and for a VM how its CPUs are
/// presented and pinned. Every setting is optional; one left out keeps the image's value or
/// leaves the domain untuned.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeCpu {
    /// The number of vCPUs, overriding the image's.
    pub count: Option<u8>,
    pub mode: Option<CpuMode>,
    pub topology: Option<CpuTopology>,
    /// The host CPU for each vCPU, in vCPU order.
    pub pinning: Option<Vec<u16>>,
    /// Host CPUs for QEMU's own threads: everything in the QEMU process except the vCPUs.
    pub emulator_pinning: Option<Vec<u16>>,
}

/// `[nodes.numa]` in the manifest: the host NUMA node a VM's memory is placed on.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeNuma {
    /// The host NUMA node the guest's memory is placed on.
    pub node: Option<u8>,
    /// How the guest's memory is held to `node`. Left out with `node` set, strict.
    pub memory_mode: Option<NumaMemoryMode>,
}

/// `[nodes.boot_disk]` in the manifest: the size of the disk a node boots from, and for a VM
/// how it is cloned and how QEMU does its I/O.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeBootDisk {
    /// The boot disk's size in GB, overriding the image's.
    pub size: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clone_method: Option<DiskImageCloneMethod>,
    pub cache: Option<DiskCache>,
    pub io: Option<DiskIo>,
    /// Gives the disks an I/O thread, pinned to these host CPUs.
    pub io_thread_cpus: Option<Vec<u16>>,
}

/// `[nodes.interfaces]` in the manifest: how many data interfaces the node has, and for a VM
/// the queues on each virtio interface.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeInterfaces {
    /// The number of data interfaces, overriding the image's.
    pub data_count: Option<u8>,
    /// virtio-net queues on each virtio interface.
    pub queues: Option<u8>,
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

/// How a guest's memory is held to its NUMA node: libvirt's `<numatune><memory mode>`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NumaMemoryMode {
    /// Only the node's memory. When the node is full, the kernel's OOM killer ends the guest.
    #[default]
    Strict,
    /// The node's memory first, then other nodes' when it is full.
    Preferred,
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

impl fmt::Display for NumaMemoryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NumaMemoryMode::Strict => write!(f, "strict"),
            NumaMemoryMode::Preferred => write!(f, "preferred"),
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

impl NodeNuma {
    /// The NUMA node the guest's memory must stay on: `node`, when the memory is held
    /// strictly. Memory that prefers a node may go to another, so it is not bound to one.
    #[must_use]
    pub fn strict_node(&self) -> Option<u8> {
        match self.memory_mode.unwrap_or_default() {
            NumaMemoryMode::Strict => self.node,
            NumaMemoryMode::Preferred => None,
        }
    }
}

impl NodeCpuPins {
    /// The pins of a node with these CPU and boot disk settings. Its QEMU threads are its
    /// emulator threads and its disk I/O thread.
    #[must_use]
    pub fn of(node_name: &str, cpu: &NodeCpu, boot_disk: &NodeBootDisk) -> Self {
        let mut qemu_thread_cpus: Vec<u16> = cpu
            .emulator_pinning
            .iter()
            .chain(boot_disk.io_thread_cpus.iter())
            .flatten()
            .copied()
            .collect();
        qemu_thread_cpus.sort_unstable();
        qemu_thread_cpus.dedup();
        Self {
            node_name: node_name.to_string(),
            vcpu_cpus: cpu.pinning.clone().unwrap_or_default(),
            qemu_thread_cpus,
        }
    }

    /// Whether the node pins nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vcpu_cpus.is_empty() && self.qemu_thread_cpus.is_empty()
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
    fn test_hardware_groups_read_every_setting() {
        let cpu: NodeCpu = serde_json::from_value(serde_json::json!({
            "count": 2,
            "mode": "host-passthrough",
            "topology": { "sockets": 1, "cores": 1, "threads": 2 },
            "pinning": [9, 25],
            "emulator_pinning": [8, 24]
        }))
        .unwrap();
        assert_eq!(cpu.count, Some(2));
        assert_eq!(cpu.mode, Some(CpuMode::HostPassthrough));
        assert_eq!(
            cpu.topology,
            Some(CpuTopology {
                sockets: 1,
                cores: 1,
                threads: 2
            })
        );
        assert_eq!(cpu.pinning, Some(vec![9, 25]));
        assert_eq!(cpu.emulator_pinning, Some(vec![8, 24]));
        let numa: NodeNuma = serde_json::from_value(serde_json::json!({
            "node": 1,
            "memory_mode": "preferred"
        }))
        .unwrap();
        assert_eq!(numa.node, Some(1));
        assert_eq!(numa.memory_mode, Some(NumaMemoryMode::Preferred));
        let disk: NodeBootDisk = serde_json::from_value(serde_json::json!({
            "size": 20,
            "clone_method": "copy",
            "cache": "none",
            "io": "io_uring",
            "io_thread_cpus": [8, 24]
        }))
        .unwrap();
        assert_eq!(disk.size, Some(20));
        assert_eq!(disk.clone_method, Some(DiskImageCloneMethod::Copy));
        assert_eq!(disk.cache, Some(DiskCache::None));
        assert_eq!(disk.io, Some(DiskIo::IoUring));
        assert_eq!(disk.io_thread_cpus, Some(vec![8, 24]));
        let interfaces: NodeInterfaces =
            serde_json::from_value(serde_json::json!({ "data_count": 4, "queues": 2 })).unwrap();
        assert_eq!(interfaces.data_count, Some(4));
        assert_eq!(interfaces.queues, Some(2));
    }

    #[test]
    fn test_numa_memory_mode_reads_strict_and_preferred() {
        for (value, mode) in [
            ("strict", NumaMemoryMode::Strict),
            ("preferred", NumaMemoryMode::Preferred),
        ] {
            let numa: NodeNuma = serde_json::from_value(serde_json::json!({
                "node": 1,
                "memory_mode": value
            }))
            .unwrap();
            assert_eq!(numa.memory_mode, Some(mode));
            assert_eq!(mode.to_string(), value);
        }
        let interleave: Result<NodeNuma, _> =
            serde_json::from_value(serde_json::json!({ "memory_mode": "interleave" }));
        assert!(interleave.is_err());
    }

    #[test]
    fn test_numa_memory_mode_defaults_to_strict() {
        assert_eq!(NumaMemoryMode::default(), NumaMemoryMode::Strict);
    }

    #[test]
    fn test_only_strict_numa_memory_is_held_to_its_node() {
        let strict = NodeNuma {
            node: Some(1),
            memory_mode: Some(NumaMemoryMode::Strict),
        };
        let preferred = NodeNuma {
            node: Some(1),
            memory_mode: Some(NumaMemoryMode::Preferred),
        };
        let unset_mode = NodeNuma {
            node: Some(1),
            memory_mode: None,
        };
        assert_eq!(strict.strict_node(), Some(1));
        assert_eq!(unset_mode.strict_node(), Some(1));
        assert_eq!(preferred.strict_node(), None);
        assert_eq!(NodeNuma::default().strict_node(), None);
    }

    #[test]
    fn test_hardware_groups_reject_unknown_settings() {
        let old: Result<NodeCpu, _> =
            serde_json::from_value(serde_json::json!({ "cpu_pinning": [1] }));
        let error = old.unwrap_err().to_string();
        assert!(error.contains("unknown field `cpu_pinning`"), "{error}");
        let old: Result<NodeBootDisk, _> =
            serde_json::from_value(serde_json::json!({ "disk_cache": "none" }));
        assert!(old.is_err());
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
    fn test_pins_join_the_emulator_and_io_thread_cpus() {
        let cpu = NodeCpu {
            pinning: Some(vec![9, 25]),
            emulator_pinning: Some(vec![24, 8]),
            ..NodeCpu::default()
        };
        let disk = NodeBootDisk {
            io_thread_cpus: Some(vec![8, 7]),
            ..NodeBootDisk::default()
        };
        let pins = NodeCpuPins::of("dev", &cpu, &disk);
        assert_eq!(pins.vcpu_cpus, vec![9, 25]);
        assert_eq!(pins.qemu_thread_cpus, vec![7, 8, 24]);
        assert!(!pins.is_empty());
        assert!(NodeCpuPins::of("dev", &NodeCpu::default(), &NodeBootDisk::default()).is_empty());
    }

    #[test]
    fn test_cpuset_lists_cpus() {
        assert_eq!(cpuset(&[8, 24]), "8,24");
        assert_eq!(cpuset(&[3]), "3");
    }
}
