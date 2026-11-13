use askama::Template;

use shared::data::{
    BiosTypes, CloneDisk, ConnectionTypes, CpuArchitecture, CpuFeature, CpuModels, CpuTopology,
    DiskBuses, DiskCache, DiskDevices, DiskIo, Interface, InterfaceType, MachineType, NodeBootDisk,
    NodeCpu, NodeDisk, NodeInterfaces, NodeNuma, NumaMemoryMode, QemuCommand, UnikernelBootMode,
    WatchdogAction, cpuset,
};

#[derive(Debug, Template)]
#[template(path = "libvirt/libvirt_domain.jinja", ext = "xml", escape = "xml")]
pub struct DomainTemplate {
    pub name: String,
    pub memory: u16,
    pub cpu_architecture: CpuArchitecture,
    pub cpu_model: CpuModels,
    pub machine_type: MachineType,
    pub cpu_count: u8,
    pub vmx_enabled: bool,
    pub qemu_bin: String,
    pub bios: BiosTypes,
    pub disks: Vec<NodeDisk>,
    pub interfaces: Vec<Interface>,
    pub interface_type: InterfaceType,
    pub management_interface_type: InterfaceType,
    pub reserved_interface_type: InterfaceType,
    pub loopback_ipv4: String,
    pub telnet_port: u16,
    pub qemu_commands: Vec<QemuCommand>,
    pub lab_id: String,
    pub management_network: String,
    pub isolated_network: String,
    pub reserved_network: String,
    pub is_windows: bool,
    pub video_model: String,
    pub video_vram: Option<u32>,
    pub cpu_features: Vec<CpuFeature>,
    /// The VNC console port on the server, reserved for the node in the database.
    pub vnc_port: u16,
    pub tuning: DomainTuning,
    /// The action of the node's hardware watchdog. None, the domain has no watchdog.
    pub watchdog: Option<WatchdogAction>,
}

impl DomainTemplate {
    /// Whether a disk is on the SCSI bus, which needs the domain's virtio-scsi controller.
    fn has_scsi_disk(&self) -> bool {
        self.disks
            .iter()
            .any(|disk| disk.target_bus == DiskBuses::Scsi)
    }
}

/// A VM's CPU, NUMA, boot disk and interface tuning, as the domain renders it. The default
/// renders no tuning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainTuning {
    pub cpu_topology: Option<CpuTopology>,
    /// The host CPU for each vCPU, in vCPU order.
    pub cpu_pinning: Vec<u16>,
    pub qemu_cpuset: Option<String>,
    /// The cpuset of the disks' I/O thread. Set, the domain has one I/O thread for its disks.
    pub io_thread_cpuset: Option<String>,
    pub numa_node: Option<u8>,
    /// How the memory is held to `numa_node`.
    pub numa_memory_mode: NumaMemoryMode,
    pub disk_cache: Option<DiskCache>,
    pub disk_io: Option<DiskIo>,
    /// Queues on the data interfaces.
    pub interface_queues: Option<u8>,
    /// Queues on the management interface, when it is virtio.
    pub management_interface_queues: Option<u8>,
}

impl DomainTuning {
    /// The domain tuning for a node's `cpu`, `numa`, `boot_disk` and `interfaces` settings.
    #[must_use]
    pub fn from_node_settings(
        cpu: &NodeCpu,
        numa: &NodeNuma,
        boot_disk: &NodeBootDisk,
        interfaces: &NodeInterfaces,
        management_interface_type: &InterfaceType,
    ) -> Self {
        let management_interface_queues = if *management_interface_type == InterfaceType::Virtio {
            interfaces.queues
        } else {
            None
        };
        Self {
            cpu_topology: cpu.topology,
            cpu_pinning: cpu.pinning.clone().unwrap_or_default(),
            qemu_cpuset: cpu.emulator_pinning.as_deref().map(cpuset),
            io_thread_cpuset: boot_disk.io_thread_cpus.as_deref().map(cpuset),
            numa_node: numa.node,
            numa_memory_mode: numa.memory_mode.unwrap_or_default(),
            disk_cache: boot_disk.cache,
            disk_io: boot_disk.io,
            interface_queues: interfaces.queues,
            management_interface_queues,
        }
    }

    /// Whether the domain pins any of its threads.
    #[must_use]
    pub fn has_cputune(&self) -> bool {
        !self.cpu_pinning.is_empty()
            || self.qemu_cpuset.is_some()
            || self.io_thread_cpuset.is_some()
    }
}

pub struct BootServer {
    pub template: DomainTemplate,
    pub copy_disks: Vec<CloneDisk>,
}

#[derive(Debug, Template)]
#[template(path = "libvirt/unikernel_domain.jinja", ext = "xml", escape = "xml")]
pub struct UnikernelDomainTemplate {
    pub name: String,
    pub memory: u16,
    pub cpu_architecture: CpuArchitecture,
    pub cpu_model: CpuModels,
    pub machine_type: MachineType,
    pub cpu_count: u8,
    pub qemu_bin: String,
    pub boot_mode: UnikernelBootMode,
    pub kernel_path: Option<String>,
    pub kernel_cmdline: Option<String>,
    pub disks: Vec<NodeDisk>,
    pub interfaces: Vec<Interface>,
    pub interface_type: InterfaceType,
    pub management_interface_type: InterfaceType,
    pub reserved_interface_type: InterfaceType,
    pub loopback_ipv4: String,
    pub telnet_port: u16,
    pub management_network: String,
    pub isolated_network: String,
    pub reserved_network: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use askama::Template;
    use shared::data::{DiskDrivers, DiskFormats, DiskTargets};

    #[test]
    fn test_domain_tuning_from_node_settings() {
        let cpu = NodeCpu {
            pinning: Some(vec![9, 25]),
            emulator_pinning: Some(vec![8, 24]),
            ..NodeCpu::default()
        };
        let numa = NodeNuma {
            node: Some(1),
            memory_mode: None,
        };
        let disk = NodeBootDisk {
            io_thread_cpus: Some(vec![8]),
            ..NodeBootDisk::default()
        };
        let interfaces = NodeInterfaces {
            data_count: None,
            queues: Some(4),
        };
        let domain = DomainTuning::from_node_settings(
            &cpu,
            &numa,
            &disk,
            &interfaces,
            &InterfaceType::Virtio,
        );
        assert_eq!(domain.cpu_pinning, vec![9, 25]);
        assert_eq!(domain.qemu_cpuset.as_deref(), Some("8,24"));
        assert_eq!(domain.io_thread_cpuset.as_deref(), Some("8"));
        assert_eq!(domain.numa_node, Some(1));
        assert_eq!(domain.numa_memory_mode, NumaMemoryMode::Strict);
        assert_eq!(domain.interface_queues, Some(4));
        assert_eq!(domain.management_interface_queues, Some(4));
        assert!(domain.has_cputune());

        // An e1000 management interface has no queues to set.
        let e1000 = DomainTuning::from_node_settings(
            &cpu,
            &numa,
            &disk,
            &interfaces,
            &InterfaceType::E1000,
        );
        assert_eq!(e1000.management_interface_queues, None);
        assert_eq!(e1000.interface_queues, Some(4));

        let untuned = DomainTuning::from_node_settings(
            &NodeCpu::default(),
            &NodeNuma::default(),
            &NodeBootDisk::default(),
            &NodeInterfaces::default(),
            &InterfaceType::Virtio,
        );
        assert_eq!(untuned, DomainTuning::default());
        assert!(!untuned.has_cputune());
    }

    #[test]
    fn test_domain_tuning_takes_the_numa_memory_mode() {
        let numa = NodeNuma {
            node: Some(0),
            memory_mode: Some(NumaMemoryMode::Preferred),
        };
        let domain = DomainTuning::from_node_settings(
            &NodeCpu::default(),
            &numa,
            &NodeBootDisk::default(),
            &NodeInterfaces::default(),
            &InterfaceType::Virtio,
        );
        assert_eq!(domain.numa_node, Some(0));
        assert_eq!(domain.numa_memory_mode, NumaMemoryMode::Preferred);
    }

    fn base_unikernel_template(boot_mode: UnikernelBootMode) -> UnikernelDomainTemplate {
        UnikernelDomainTemplate {
            name: "test-uk-lab1".to_string(),
            memory: 512,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostModel,
            machine_type: MachineType::Pc,
            cpu_count: 1,
            qemu_bin: "/usr/bin/qemu-system-x86_64".to_string(),
            boot_mode,
            kernel_path: None,
            kernel_cmdline: None,
            disks: vec![],
            interfaces: vec![],
            interface_type: InterfaceType::Virtio,
            management_interface_type: InterfaceType::Virtio,
            reserved_interface_type: InterfaceType::Virtio,
            loopback_ipv4: "127.0.0.1".to_string(),
            telnet_port: 5000,
            management_network: "mgmt-lab1".to_string(),
            isolated_network: "iso-lab1".to_string(),
            reserved_network: "rsv-lab1".to_string(),
        }
    }

    #[test]
    fn test_unikernel_direct_kernel_renders_kernel_element() {
        let mut tmpl = base_unikernel_template(UnikernelBootMode::DirectKernel);
        tmpl.kernel_path = Some("/images/unikraft/v1/app.elf".to_string());
        tmpl.kernel_cmdline = Some("netdev.ipv4_addr=10.0.0.10/24".to_string());

        let xml = tmpl.render().unwrap();
        assert!(xml.contains("<kernel>/images/unikraft/v1/app.elf</kernel>"));
        assert!(xml.contains("<cmdline>netdev.ipv4_addr=10.0.0.10/24</cmdline>"));
        assert!(!xml.contains("<boot dev="));
    }

    #[test]
    fn test_unikernel_disk_boot_renders_boot_dev() {
        let mut tmpl = base_unikernel_template(UnikernelBootMode::DiskBoot);
        tmpl.disks = vec![NodeDisk {
            driver_name: DiskDrivers::Qemu,
            driver_format: DiskFormats::Qcow2,
            src_file: "/pool/test-uk-lab1.qcow2".to_string(),
            target_dev: DiskTargets::Vda,
            target_bus: DiskBuses::Virtio,
            disk_device: DiskDevices::File,
        }];

        let xml = tmpl.render().unwrap();
        assert!(xml.contains("<boot dev='hd'/>"));
        assert!(!xml.contains("<kernel>"));
        assert!(!xml.contains("<cmdline>"));
        assert!(xml.contains("/pool/test-uk-lab1.qcow2"));
    }

    #[test]
    fn test_unikernel_template_has_no_usb_vnc_watchdog() {
        let tmpl = base_unikernel_template(UnikernelBootMode::DirectKernel);
        let xml = tmpl.render().unwrap();

        assert!(!xml.contains("<controller type='usb'"));
        assert!(!xml.contains("<graphics"));
        assert!(!xml.contains("<video"));
        assert!(!xml.contains("<watchdog"));
        assert!(!xml.contains("<channel"));
        assert!(!xml.contains("smbios"));
        assert!(!xml.contains("memballoon"));
    }

    #[test]
    fn test_unikernel_template_has_serial_console() {
        let tmpl = base_unikernel_template(UnikernelBootMode::DirectKernel);
        let xml = tmpl.render().unwrap();

        assert!(xml.contains("<serial type='tcp'>"));
        assert!(xml.contains("<console type='tcp'>"));
        assert!(xml.contains("service='5000'"));
        assert!(xml.contains("host='127.0.0.1'"));
    }

    #[test]
    fn test_unikernel_template_has_acpi() {
        let tmpl = base_unikernel_template(UnikernelBootMode::DirectKernel);
        let xml = tmpl.render().unwrap();
        assert!(xml.contains("<acpi/>"));
    }

    #[test]
    fn test_unikernel_template_renders_management_interface() {
        let mut tmpl = base_unikernel_template(UnikernelBootMode::DirectKernel);
        tmpl.interfaces = vec![Interface {
            name: "mgmt0".to_string(),
            num: 0,
            mtu: 9500,
            mac_address: "52:54:00:aa:bb:cc".to_string(),
            connection_type: ConnectionTypes::Management,
        }];

        let xml = tmpl.render().unwrap();
        assert!(xml.contains("<source network='mgmt-lab1'/>"));
        assert!(xml.contains("52:54:00:aa:bb:cc"));
    }

    #[test]
    fn test_unikernel_p2p_interface_sets_its_mtu() {
        let mut tmpl = base_unikernel_template(UnikernelBootMode::DirectKernel);
        tmpl.interfaces = vec![Interface {
            name: "tpa0-lab1".to_string(),
            num: 1,
            mtu: 9600,
            mac_address: "52:54:00:aa:bb:cc".to_string(),
            connection_type: ConnectionTypes::P2p,
        }];

        let xml = tmpl.render().unwrap();
        let p2p = &xml[xml.find("<interface type='ethernet'>").unwrap()..];
        let p2p = &p2p[..p2p.find("</interface>").unwrap()];
        assert!(p2p.contains("<target dev='tpa0-lab1'/>"), "{p2p}");
        assert!(p2p.contains("<mtu size='9600'/>"), "{p2p}");
    }

    #[test]
    fn test_unikernel_cpu_has_no_arch_element() {
        let xml = base_unikernel_template(UnikernelBootMode::DirectKernel)
            .render()
            .unwrap();
        // libvirt's domain schema has no <arch> under <cpu>; the guest
        // architecture is set on <os><type>.
        assert!(!xml.contains("<arch>"), "{xml}");
        assert!(xml.contains("<type arch='x86_64'"), "{xml}");
    }
}
