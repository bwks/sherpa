use std::fmt;

use askama::Template;

use shared::data::{
    BiosTypes, ConnectionTypes, CpuArchitecture, CpuFeature, CpuModels, CpuTopology, DiskBuses,
    DiskCache, DiskDevices, DiskDrivers, DiskFormats, DiskIo, DiskTargets, Interface,
    InterfaceType, MachineType, NodeBootDisk, NodeCpu, NodeDisk, NodeInterfaces, NodeNuma,
    NumaMemoryMode, QemuCommand, UnikernelBootMode, WatchdogAction, cpuset,
};

use crate::sandbox::SANDBOX_IMAGE_ARCHIVE_SERIAL;

/// A virtio-serial channel of a domain: the QEMU guest agent's, or the sandbox status channel
/// bound to its socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Channel {
    GuestAgent,
    Status(String),
}

/// The SMBIOS system serial a domain gives its guest: `ds=nocloud`, which points cloud-init at
/// its `NoCloud` seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmbiosSerial {
    NoCloud,
}

/// The host source of a domain's virtio RNG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RngBackend {
    Urandom,
}

/// A domain's video device model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoModel {
    Cirrus,
    Virtio,
    None,
}

/// A domain's USB controller: QEMU's xHCI, or none, without which libvirt adds one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbController {
    QemuXhci,
    None,
}

/// A domain's memory balloon model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemballoonModel {
    Virtio,
    None,
}

/// What libvirt does with a domain whose guest reboots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebootAction {
    Restart,
    Destroy,
}

/// A device a domain's firmware boots from, in the domain's boot order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootDevice {
    Cdrom,
    Hd,
}

/// The domain of a VM: a VM node, a sandbox, or a VM that makes a sandbox's boot disk or Docker
/// store.
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
    pub boot_devices: Vec<BootDevice>,
    pub on_reboot: RebootAction,
    pub smbios_serial: Option<SmbiosSerial>,
    pub disks: Vec<NodeDisk>,
    pub interfaces: Vec<Interface>,
    pub sandbox_nic_mac: Option<String>,
    pub interface_type: InterfaceType,
    pub management_interface_type: InterfaceType,
    pub reserved_interface_type: InterfaceType,
    pub serial_socket: Option<String>,
    pub channels: Vec<Channel>,
    pub usb_controller: UsbController,
    pub rng: Option<RngBackend>,
    pub qemu_commands: Vec<QemuCommand>,
    pub ignition_config: Option<String>,
    pub lab_id: String,
    pub management_network: String,
    pub isolated_network: String,
    pub reserved_network: String,
    pub is_windows: bool,
    pub video_model: VideoModel,
    pub video_vram: Option<u32>,
    pub memballoon: MemballoonModel,
    pub cpu_features: Vec<CpuFeature>,
    pub vnc_socket: Option<String>,
    pub tuning: DomainTuning,
    pub watchdog: Option<WatchdogAction>,
}

impl fmt::Display for SmbiosSerial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoCloud => "ds=nocloud",
        })
    }
}

impl fmt::Display for RngBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Urandom => "/dev/urandom",
        })
    }
}

impl fmt::Display for VideoModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cirrus => "cirrus",
            Self::Virtio => "virtio",
            Self::None => "none",
        })
    }
}

impl fmt::Display for MemballoonModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Virtio => "virtio",
            Self::None => "none",
        })
    }
}

impl fmt::Display for RebootAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Restart => "restart",
            Self::Destroy => "destroy",
        })
    }
}

impl fmt::Display for BootDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cdrom => "cdrom",
            Self::Hd => "hd",
        })
    }
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
    pub cpu_pinning: Vec<u16>,
    pub qemu_cpuset: Option<String>,
    pub io_thread_cpuset: Option<String>,
    pub numa_node: Option<u8>,
    pub numa_memory_mode: NumaMemoryMode,
    pub disk_cache: Option<DiskCache>,
    pub disk_io: Option<DiskIo>,
    pub interface_queues: Option<u8>,
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

/// A Flatcar VM Sherpa runs for a node, a sandbox or a VM that makes a sandbox's boot disk or
/// Docker store: booted through Flatcar's bootloader from `disk`, with its config drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatcarDomain {
    pub name: String,
    pub memory: u16,
    pub vcpus: u8,
    pub qemu_bin: String,
    pub disk: String,
    pub config_drive: String,
}

/// What a sandbox has beyond a Flatcar VM: its Docker disk, its NICs, the management NIC first,
/// the NIC of its own on the lab's management network, its serial console and status sockets,
/// and its node's tuning.
#[derive(Debug)]
pub struct SandboxDomain {
    pub docker_disk: String,
    pub lab_id: String,
    pub management_network: String,
    pub interfaces: Vec<Interface>,
    pub own_mac: String,
    pub serial_socket: String,
    pub status_socket: String,
    pub tuning: DomainTuning,
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
    pub serial_socket: String,
    pub management_network: String,
    pub isolated_network: String,
    pub reserved_network: String,
}

impl FlatcarDomain {
    /// The domain of the VM that makes the sandboxes' boot disk, which its first reboot stops.
    #[must_use]
    pub fn boot_vm(self) -> DomainTemplate {
        let mut domain = self.domain(Vec::new());
        domain.on_reboot = RebootAction::Destroy;
        domain
    }

    /// The domain of the VM that makes a Docker store on the empty disk `docker_disk` from the
    /// image archive `archive`, a read-only disk it finds by its serial, and reports on its status
    /// channel at `status_socket`.
    #[must_use]
    pub fn store_vm(
        self,
        docker_disk: String,
        archive: String,
        status_socket: String,
    ) -> DomainTemplate {
        let mut domain = self.domain(vec![
            virtio_disk(docker_disk, DiskTargets::Vdb),
            NodeDisk {
                driver_format: DiskFormats::Raw,
                serial: Some(SANDBOX_IMAGE_ARCHIVE_SERIAL.to_owned()),
                read_only: true,
                ..virtio_disk(archive, DiskTargets::Vdc)
            },
        ]);
        domain.channels = vec![Channel::Status(status_socket)];
        domain
    }

    /// The domain of a sandbox.
    #[must_use]
    pub fn sandbox(self, sandbox: SandboxDomain) -> DomainTemplate {
        let mut domain = self.domain(vec![virtio_disk(sandbox.docker_disk, DiskTargets::Vdb)]);
        domain.lab_id = sandbox.lab_id;
        domain.management_network = sandbox.management_network;
        domain.interfaces = sandbox.interfaces;
        domain.sandbox_nic_mac = Some(sandbox.own_mac);
        domain.serial_socket = Some(sandbox.serial_socket);
        domain.channels = vec![Channel::Status(sandbox.status_socket)];
        domain.tuning = sandbox.tuning;
        domain
    }

    /// The VM's domain with its disk, `disks` and its config drive, booting its disk only, so
    /// the firmware never tries the config drive: no NICs, console, status channel, VNC, guest
    /// agent, USB controller, `ds=nocloud` sysinfo, video or memory balloon, and no tuning.
    fn domain(self, disks: Vec<NodeDisk>) -> DomainTemplate {
        let mut all = vec![virtio_disk(self.disk, DiskTargets::Vda)];
        all.extend(disks);
        all.push(NodeDisk {
            disk_device: DiskDevices::Cdrom,
            driver_name: DiskDrivers::Qemu,
            driver_format: DiskFormats::Raw,
            src_file: self.config_drive,
            target_dev: DiskTargets::Sda,
            target_bus: DiskBuses::Sata,
            serial: None,
            read_only: true,
        });
        DomainTemplate {
            name: self.name,
            memory: self.memory,
            cpu_architecture: CpuArchitecture::X86_64,
            cpu_model: CpuModels::HostPassthrough,
            machine_type: MachineType::Q35,
            cpu_count: self.vcpus,
            vmx_enabled: false,
            qemu_bin: self.qemu_bin,
            bios: BiosTypes::SeaBios,
            boot_devices: vec![BootDevice::Hd],
            on_reboot: RebootAction::Restart,
            smbios_serial: None,
            disks: all,
            interfaces: Vec::new(),
            sandbox_nic_mac: None,
            interface_type: InterfaceType::Virtio,
            management_interface_type: InterfaceType::Virtio,
            reserved_interface_type: InterfaceType::Virtio,
            serial_socket: None,
            channels: Vec::new(),
            usb_controller: UsbController::None,
            rng: Some(RngBackend::Urandom),
            qemu_commands: Vec::new(),
            ignition_config: None,
            lab_id: String::new(),
            management_network: String::new(),
            isolated_network: String::new(),
            reserved_network: String::new(),
            is_windows: false,
            video_model: VideoModel::None,
            video_vram: None,
            memballoon: MemballoonModel::None,
            cpu_features: Vec::new(),
            vnc_socket: None,
            tuning: DomainTuning::default(),
            watchdog: None,
        }
    }
}

/// A QCOW2 disk at `path`, as `target` on the virtio bus.
fn virtio_disk(path: String, target: DiskTargets) -> NodeDisk {
    NodeDisk {
        disk_device: DiskDevices::File,
        driver_name: DiskDrivers::Qemu,
        driver_format: DiskFormats::Qcow2,
        src_file: path,
        target_dev: target,
        target_bus: DiskBuses::Virtio,
        serial: None,
        read_only: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use askama::Template;
    use shared::data::{DiskDrivers, DiskFormats, DiskTargets};

    #[test]
    fn vm_tuning_019_a_nodes_settings_become_its_domains_tuning_and_none_gives_none() {
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
            serial_socket: "/opt/sherpa/run/console/lab1/1.serial".to_string(),
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
            serial: None,
            read_only: false,
        }];

        let xml = tmpl.render().unwrap();
        assert!(xml.contains("<boot dev='hd'/>"));
        assert!(!xml.contains("<kernel>"));
        assert!(!xml.contains("<cmdline>"));
        assert!(xml.contains("/pool/test-uk-lab1.qcow2"));
    }

    #[test]
    fn console_sockets_004_a_unikernel_is_defined_with_its_serial_socket_and_no_vnc_console() {
        let tmpl = base_unikernel_template(UnikernelBootMode::DirectKernel);
        let xml = tmpl.render().unwrap();

        let source = "<source mode='bind' path='/opt/sherpa/run/console/lab1/1.serial'/>";
        assert!(xml.contains(&format!("<serial type='unix'>\n      {source}")));
        assert!(xml.contains(&format!("<console type='unix'>\n      {source}")));
        assert!(!xml.contains("<graphics"), "{xml}");
    }

    #[test]
    fn management_network_007_a_unikernels_management_interface_is_on_the_management_network() {
        let mut tmpl = base_unikernel_template(UnikernelBootMode::DirectKernel);
        tmpl.interfaces = vec![Interface {
            name: "mgmt0".to_string(),
            num: 0,
            mtu: 9500,
            mac_address: "52:54:00:aa:bb:cc".to_string(),
            connection_type: ConnectionTypes::Management,
            tap: None,
        }];

        let xml = tmpl.render().unwrap();
        assert!(xml.contains("<source network='mgmt-lab1'/>"));
        assert!(xml.contains("52:54:00:aa:bb:cc"));
    }

    #[test]
    fn p2p_link_016_a_unikernels_link_interface_has_the_links_mtu() {
        let mut tmpl = base_unikernel_template(UnikernelBootMode::DirectKernel);
        tmpl.interfaces = vec![Interface {
            name: "tpa0-lab1".to_string(),
            num: 1,
            mtu: 9600,
            mac_address: "52:54:00:aa:bb:cc".to_string(),
            connection_type: ConnectionTypes::P2p,
            tap: None,
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
