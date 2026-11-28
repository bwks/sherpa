use askama::Template;

use shared::data::{
    BiosTypes, ConnectionTypes, CpuArchitecture, CpuModels, CpuTopology, DiskBuses, DiskCache,
    DiskDevices, DiskDrivers, DiskFormats, DiskIo, DiskTargets, Interface, InterfaceType,
    MachineType, NodeBootDisk, NodeCpu, NodeDisk, NodeInterfaces, NodeNuma, NumaMemoryMode,
    QemuCommand, WatchdogAction,
};
use template::{
    BootDevice, Channel, DomainTemplate, DomainTuning, MemballoonModel, SmbiosSerial,
    UsbController, VideoModel,
};

// ============================================================================
// Helpers
// ============================================================================

fn base_domain_template() -> DomainTemplate {
    DomainTemplate {
        name: "dev01-a10736e8".to_string(),
        memory: 2048,
        cpu_architecture: CpuArchitecture::X86_64,
        cpu_model: CpuModels::HostModel,
        machine_type: MachineType::Q35,
        cpu_count: 2,
        vmx_enabled: false,
        qemu_bin: "/usr/bin/qemu-system-x86_64".to_string(),
        bios: BiosTypes::SeaBios,
        boot_devices: vec![BootDevice::Cdrom, BootDevice::Hd],
        smbios_serial: Some(SmbiosSerial::NoCloud),
        disks: vec![NodeDisk {
            disk_device: DiskDevices::File,
            driver_name: DiskDrivers::Qemu,
            driver_format: DiskFormats::Qcow2,
            src_file: "/opt/sherpa/labs/a10736e8/dev01.qcow2".to_string(),
            target_dev: DiskTargets::Vda,
            target_bus: DiskBuses::Virtio,
            serial: None,
            read_only: false,
        }],
        interfaces: vec![Interface {
            name: "mgmt".to_string(),
            num: 0,
            mtu: 1500,
            mac_address: "52:54:00:aa:bb:cc".to_string(),
            connection_type: ConnectionTypes::Management,
            tap: None,
        }],
        sandbox_nic_mac: None,
        interface_type: InterfaceType::Virtio,
        management_interface_type: InterfaceType::Virtio,
        reserved_interface_type: InterfaceType::Virtio,
        serial_socket: Some("/opt/sherpa/run/console/a10736e8/1.serial".to_string()),
        channels: vec![Channel::GuestAgent],
        usb_controller: UsbController::QemuXhci,
        rng: None,
        qemu_commands: vec![],
        ignition_config: None,
        lab_id: "a10736e8".to_string(),
        management_network: "sherpa-management-a10736e8".to_string(),
        isolated_network: String::new(),
        reserved_network: String::new(),
        is_windows: false,
        video_model: VideoModel::Cirrus,
        video_vram: Some(16384),
        memballoon: MemballoonModel::Virtio,
        cpu_features: vec![],
        vnc_socket: Some("/opt/sherpa/run/console/a10736e8/1.vnc".to_string()),
        tuning: DomainTuning::default(),
        watchdog: None,
    }
}

fn p2p_interface() -> Interface {
    Interface {
        name: "tpa0-a10736e8".to_string(),
        num: 1,
        mtu: 9600,
        mac_address: "52:54:00:aa:bb:01".to_string(),
        connection_type: ConnectionTypes::P2p,
        tap: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn console_sockets_003_a_vm_is_defined_with_both_consoles_on_its_own_sockets() {
    let xml = base_domain_template().render().expect("renders domain XML");
    // libvirt binds the serial socket and QEMU the VNC one; only sherpad connects to them.
    let source = "<source mode='bind' path='/opt/sherpa/run/console/a10736e8/1.serial'/>";
    assert!(
        xml.contains(&format!("<serial type='unix'>\n      {source}")),
        "{xml}"
    );
    assert!(
        xml.contains(&format!("<console type='unix'>\n      {source}")),
        "{xml}"
    );
    assert!(
        xml.contains(
            "<graphics type='vnc'>\n      \
             <listen type='socket' socket='/opt/sherpa/run/console/a10736e8/1.vnc'/>\n    \
             </graphics>"
        ),
        "{xml}"
    );
}

#[test]
fn test_domain_has_a_virtio_scsi_controller_only_for_scsi_disks() {
    let xml = base_domain_template().render().unwrap();
    assert!(!xml.contains("<controller type='scsi'"), "{xml}");

    let mut template = base_domain_template();
    template.disks[0].target_dev = DiskTargets::Sda;
    template.disks[0].target_bus = DiskBuses::Scsi;
    let xml = template.render().unwrap();
    assert!(xml.contains("<target dev='sda' bus='scsi'/>"), "{xml}");
    assert!(
        xml.contains("<controller type='scsi' index='0' model='virtio-scsi'/>"),
        "{xml}"
    );

    // libvirt takes an I/O thread only on a virtio disk, so a SCSI disk's is on its controller.
    template.tuning.io_thread_cpuset = Some("2".to_string());
    let xml = template.render().unwrap();
    assert!(
        xml.contains("<driver name='qemu' type='qcow2'\n      />"),
        "{xml}"
    );
    assert!(
        xml.contains(
            "<controller type='scsi' index='0' model='virtio-scsi'>\n      <driver iothread='1'/>"
        ),
        "{xml}"
    );
}

#[test]
fn config_delivery_015_a_domain_has_a_firmware_config_entry_only_with_an_ignition_config() {
    let mut template = base_domain_template();
    template.ignition_config =
        Some("/opt/sherpa/libvirt/images/flatcar01-a10736e8-cfg.ign".to_string());
    let xml = template.render().unwrap();
    // libvirt labels the file and adds it to the VM's AppArmor profile, which it does not do
    // for a raw `-fw_cfg` QEMU argument.
    assert!(
        xml.contains(
            "<sysinfo type='fwcfg'>\n    <entry name='opt/org.flatcar-linux/config' \
             file='/opt/sherpa/libvirt/images/flatcar01-a10736e8-cfg.ign'/>\n  </sysinfo>"
        ),
        "{xml}"
    );
    assert!(!xml.contains("-fw_cfg"), "{xml}");
    assert!(!xml.contains("<ns0:commandline>"), "{xml}");

    let xml = base_domain_template().render().unwrap();
    assert!(!xml.contains("fwcfg"), "{xml}");
    assert!(!xml.contains("org.flatcar-linux"), "{xml}");
}

#[test]
fn test_domain_keeps_the_qemu_arguments_of_other_models() {
    let mut template = base_domain_template();
    template.qemu_commands = QemuCommand::juniper_router();
    let xml = template.render().unwrap();
    assert!(
        xml.contains(
            "<ns0:commandline>\n    <ns0:arg value='-smbios'/>\n    \
             <ns0:arg value='type=1,product=VM-VMX,family=lab'/>\n  </ns0:commandline>"
        ),
        "{xml}"
    );
    assert!(!xml.contains("fwcfg"), "{xml}");
}

#[test]
fn vm_tuning_024_a_domain_has_a_watchdog_only_when_its_node_sets_one() {
    // Cisco IOS images arm an i6300esb watchdog early in boot and feed it only after a long init
    // phase; when that phase outlasts the timeout, as it does on every boot under nested KVM, a
    // reset-action watchdog restarts the VM in a loop. So a VM has one only when its node asks.
    let xml = base_domain_template().render().unwrap();
    assert!(!xml.contains("<watchdog"), "{xml}");

    let mut domain = base_domain_template();
    domain.watchdog = Some(WatchdogAction::InjectNmi);
    let xml = domain.render().unwrap();
    assert!(
        xml.contains("<watchdog model='i6300esb' action='inject-nmi'/>"),
        "{xml}"
    );
    assert_eq!(xml.matches("<watchdog").count(), 1, "{xml}");
}

#[test]
fn node_interfaces_005_an_unused_interface_of_a_vm_has_its_link_down() {
    let mut domain = base_domain_template();
    domain.isolated_network = "sherpa-isolated-dev01-a10736e8".to_string();
    domain.interfaces.push(Interface {
        name: "eth1".to_string(),
        num: 1,
        mtu: 1500,
        mac_address: "52:54:00:dd:ee:ff".to_string(),
        connection_type: ConnectionTypes::Disabled,
        tap: None,
    });

    let output = domain
        .render()
        .expect("domain template should render with disabled interface");
    assert!(output.contains("sherpa-isolated-dev01-a10736e8"));
    assert!(output.contains("link state='down'"));

    // Reproduces the bug where a VM with no disabled interfaces has no
    // isolated network, causing `sherpa up` to fail with:
    //   "Isolated network not found for VM node: dev01"
    let domain = base_domain_template();
    let output = domain
        .render()
        .expect("domain template should render with empty isolated_network");
    assert!(output.contains("<name>dev01-a10736e8</name>"));
    assert!(output.contains("sherpa-management-a10736e8"));
    // isolated_network should NOT appear in output since there are no disabled interfaces
    assert!(!output.contains("disabled"));
}

#[test]
fn p2p_link_004_a_vm_is_told_its_links_mtu() {
    let mut domain = base_domain_template();
    domain.interfaces.push(Interface {
        name: "tpa0-a10736e8".to_string(),
        num: 1,
        mtu: 9600,
        mac_address: "52:54:00:dd:ee:ff".to_string(),
        connection_type: ConnectionTypes::P2p,
        tap: None,
    });

    let xml = domain.render().expect("renders domain XML");
    let start = xml
        .find("<interface type='ethernet'>")
        .expect("renders a P2p interface");
    let p2p = &xml[start..];
    let p2p = &p2p[..p2p.find("</interface>").expect("closes the P2p interface")];
    assert!(p2p.contains("<target dev='tpa0-a10736e8'/>"), "{p2p}");
    assert!(p2p.contains("<mtu size='9600'/>"), "{p2p}");
}

#[test]
fn test_domain_windows_has_full_hyperv_enlightenments() {
    let mut domain = base_domain_template();
    domain.is_windows = true;
    let xml = domain.render().expect("renders domain XML");
    for element in [
        "<relaxed state='on'/>",
        "<vapic state='on'/>",
        "<spinlocks state='on' retries='8191'/>",
        "<vpindex state='on'/>",
        "<runtime state='on'/>",
        "<synic state='on'/>",
        "<stimer state='on'/>",
        "<frequencies state='on'/>",
        "<tlbflush state='on'/>",
        "<ipi state='on'/>",
    ] {
        assert!(xml.contains(element), "missing {element}: {xml}");
    }
    // stimer needs the hypervclock timer.
    assert!(
        xml.contains("<timer name='hypervclock' present='yes'/>"),
        "{xml}"
    );
}

#[test]
fn test_domain_non_windows_has_no_hyperv_enlightenments() {
    let xml = base_domain_template().render().expect("renders domain XML");
    assert!(!xml.contains("<hyperv"), "{xml}");
    assert!(!xml.contains("hypervclock"), "{xml}");
}

#[test]
fn test_domain_cpu_has_no_arch_element() {
    let xml = base_domain_template().render().expect("renders domain XML");
    // libvirt's domain schema has no <arch> under <cpu>; the guest
    // architecture is set on <os><type>.
    assert!(!xml.contains("<arch>"), "{xml}");
    assert!(xml.contains("<type arch='x86_64'"), "{xml}");
}

#[test]
fn vm_tuning_017_a_domain_defined_with_no_tuning_has_no_tuning_element() {
    let xml = base_domain_template().render().expect("renders domain XML");
    for element in [
        "<cputune>",
        "<numatune>",
        "<iothreads>",
        "<topology",
        "queues=",
        "cache=",
        "io=",
        "iothread=",
    ] {
        assert!(!xml.contains(element), "{element} in {xml}");
    }
}

#[test]
fn vm_tuning_018_pinning_gives_the_domains_vcpu_pins_emulator_pin_and_one_pinned_io_thread() {
    let mut domain = base_domain_template();
    domain.tuning = DomainTuning {
        cpu_pinning: vec![9, 25],
        qemu_cpuset: Some("8,24".to_string()),
        io_thread_cpuset: Some("8,24".to_string()),
        ..DomainTuning::default()
    };
    let xml = domain.render().expect("renders domain XML");
    assert!(xml.contains("<iothreads>1</iothreads>"), "{xml}");
    assert!(xml.contains("<vcpupin vcpu='0' cpuset='9'/>"), "{xml}");
    assert!(xml.contains("<vcpupin vcpu='1' cpuset='25'/>"), "{xml}");
    assert!(xml.contains("<emulatorpin cpuset='8,24'/>"), "{xml}");
    assert!(
        xml.contains("<iothreadpin iothread='1' cpuset='8,24'/>"),
        "{xml}"
    );
    assert!(
        xml.contains("<driver name='qemu' type='qcow2'\n        iothread='1'\n      />"),
        "{xml}"
    );
}

#[test]
fn vm_tuning_020_a_numa_node_holds_the_domains_memory_strictly_or_as_the_memory_mode_says() {
    let held = |memory_mode| {
        let numa = NodeNuma {
            node: Some(1),
            memory_mode,
        };
        let mut domain = base_domain_template();
        domain.tuning = DomainTuning::from_node_settings(
            &NodeCpu::default(),
            &numa,
            &NodeBootDisk::default(),
            &NodeInterfaces::default(),
            &InterfaceType::Virtio,
        );
        domain.render().expect("renders domain XML")
    };

    let xml = held(None);
    assert!(
        xml.contains("<numatune>\n    <memory mode='strict' nodeset='1'/>\n  </numatune>"),
        "{xml}"
    );
    let xml = held(Some(NumaMemoryMode::Preferred));
    assert!(
        xml.contains("<numatune>\n    <memory mode='preferred' nodeset='1'/>\n  </numatune>"),
        "{xml}"
    );
}

#[test]
fn vm_tuning_021_host_passthrough_and_topology_are_the_domains_cpu_mode_and_topology() {
    let mut domain = base_domain_template();
    domain.cpu_model = CpuModels::HostPassthrough;
    domain.tuning.cpu_topology = Some(CpuTopology {
        sockets: 1,
        cores: 1,
        threads: 2,
    });
    let xml = domain.render().expect("renders domain XML");
    assert!(xml.contains("<cpu mode='host-passthrough'>"), "{xml}");
    assert!(
        xml.contains("<topology sockets='1' dies='1' cores='1' threads='2'/>"),
        "{xml}"
    );
}

#[test]
fn vm_tuning_022_boot_disk_cache_and_io_are_on_the_domains_disk_driver() {
    let mut domain = base_domain_template();
    domain.tuning.disk_cache = Some(DiskCache::None);
    domain.tuning.disk_io = Some(DiskIo::IoUring);
    let xml = domain.render().expect("renders domain XML");
    assert!(
        xml.contains(
            "<driver name='qemu' type='qcow2'\n        cache='none'\n        io='io_uring'\n      />"
        ),
        "{xml}"
    );
}

#[test]
fn vm_tuning_023_interface_queues_are_the_queue_count_of_each_virtio_interface() {
    let mut domain = base_domain_template();
    domain.interfaces.push(p2p_interface());
    domain.tuning.interface_queues = Some(4);
    domain.tuning.management_interface_queues = Some(4);
    let xml = domain.render().expect("renders domain XML");
    assert_eq!(xml.matches("<driver queues='4'/>").count(), 2, "{xml}");
}
