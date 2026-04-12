use askama::Template;

use shared::data::{
    BiosTypes, CloneDisk, ConnectionTypes, CpuArchitecture, CpuFeature, CpuModels, DiskBuses,
    DiskDevices, Interface, InterfaceType, MachineType, NodeDisk, QemuCommand, UnikernelBootMode,
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
    pub cpu_features: Vec<CpuFeature>,
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
            interface_connection: None,
        }];

        let xml = tmpl.render().unwrap();
        assert!(xml.contains("<source network='mgmt-lab1'/>"));
        assert!(xml.contains("52:54:00:aa:bb:cc"));
    }
}
