use askama::Template;

use shared::data::{
    BiosTypes, ConnectionTypes, CpuArchitecture, CpuModels, DiskBuses, DiskDevices, DiskDrivers,
    DiskFormats, DiskTargets, Interface, InterfaceConnection, InterfaceType, MachineType, NodeDisk,
};
use template::DomainTemplate;

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
        disks: vec![NodeDisk {
            disk_device: DiskDevices::File,
            driver_name: DiskDrivers::Qemu,
            driver_format: DiskFormats::Qcow2,
            src_file: "/opt/sherpa/labs/a10736e8/dev01.qcow2".to_string(),
            target_dev: DiskTargets::Vda,
            target_bus: DiskBuses::Virtio,
        }],
        interfaces: vec![Interface {
            name: "mgmt".to_string(),
            num: 0,
            mtu: 1500,
            mac_address: "52:54:00:aa:bb:cc".to_string(),
            connection_type: ConnectionTypes::Management,
            interface_connection: None,
        }],
        interface_type: InterfaceType::Virtio,
        management_interface_type: InterfaceType::Virtio,
        reserved_interface_type: InterfaceType::Virtio,
        loopback_ipv4: "127.0.0.10".to_string(),
        telnet_port: 5000,
        qemu_commands: vec![],
        lab_id: "a10736e8".to_string(),
        management_network: "sherpa-management-a10736e8".to_string(),
        isolated_network: String::new(),
        reserved_network: String::new(),
        is_windows: false,
        video_model: "cirrus".to_owned(),
        video_vram: Some(16384),
        cpu_features: vec![],
        vnc_port: 5905,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_domain_vnc_console_has_a_fixed_port() {
    let xml = base_domain_template().render().expect("renders domain XML");
    // The node's port is reserved in the database, so libvirt must not pick another.
    assert!(
        xml.contains("<graphics type='vnc' port='5905' autoport='no'/>"),
        "{xml}"
    );
    assert!(!xml.contains("autoport='yes'"), "{xml}");
}

#[test]
fn test_domain_renders_without_isolated_network() {
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
fn test_domain_renders_with_disabled_interface() {
    let mut domain = base_domain_template();
    domain.isolated_network = "sherpa-isolated-dev01-a10736e8".to_string();
    domain.interfaces.push(Interface {
        name: "eth1".to_string(),
        num: 1,
        mtu: 1500,
        mac_address: "52:54:00:dd:ee:ff".to_string(),
        connection_type: ConnectionTypes::Disabled,
        interface_connection: None,
    });

    let output = domain
        .render()
        .expect("domain template should render with disabled interface");
    assert!(output.contains("sherpa-isolated-dev01-a10736e8"));
    assert!(output.contains("link state='down'"));
}

#[test]
fn test_domain_renders_with_peer_interface() {
    let mut domain = base_domain_template();
    domain.interfaces.push(Interface {
        name: "eth1".to_string(),
        num: 1,
        mtu: 1500,
        mac_address: "52:54:00:dd:ee:ff".to_string(),
        connection_type: ConnectionTypes::Peer,
        interface_connection: Some(InterfaceConnection {
            local_id: 0,
            local_port: 10001,
            local_loopback: "127.0.0.10".to_string(),
            source_id: 1,
            source_port: 10002,
            source_loopback: "127.0.0.11".to_string(),
        }),
    });

    let output = domain
        .render()
        .expect("domain template should render with peer interface");
    assert!(output.contains("<name>dev01-a10736e8</name>"));
    // No isolated network referenced
    assert!(!output.contains("isolated"));
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
