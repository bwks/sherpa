use std::io;

use anyhow::{Context, Result};
use quick_xml::Writer;
use quick_xml::events::BytesText;
use tracing::instrument;
use virt::connect::Connect;
use virt::domain::Domain;
use virt::error::ErrorNumber;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;
use virt::sys::VIR_DOMAIN_RUNNING;

use shared::konst::SHERPA_STORAGE_POOL;

use crate::error::LibvirtError;
use crate::storage::refresh_pool;

/// The guest kernel's command line: the serial console, the initramfs init, and a guest whose
/// init ended stops at once instead of hanging.
const KERNEL_CMDLINE: &str = "console=ttyS0 rdinit=/init panic=-1 reboot=t";
/// The virtio-fs tag of the image's root filesystem.
const ROOTFS_TAG: &str = "rootfs";
/// The virtio-fs tag of the node's own files.
const NODE_TAG: &str = "node";

/// A container image run as the root of a direct-boot QEMU guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerVm {
    pub name: String,
    pub memory_mib: u64,
    pub vcpus: u8,
    pub kernel: String,
    pub initrd: String,
    pub rootfs_dir: String,
    pub node_dir: String,
    pub disk: String,
    pub management_network: String,
    pub management_mac: String,
    pub nics: Vec<ContainerVmNic>,
    pub serial_socket: String,
    pub serial_log: String,
}

/// A container VM's P2p interface: a tap the host sees under `tap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerVmNic {
    pub tap: String,
    pub mac: String,
    pub up: bool,
}

impl ContainerVm {
    /// The domain XML of the container VM: q35 with no video, balloon or USB, booted straight
    /// into the kernel and initramfs, with the rootfs and node shares on virtio-fs, the node's
    /// raw disk on virtio, the management interface on the lab's management network, a tap per P2p interface, and the
    /// serial console on a Unix socket.
    ///
    /// # Errors
    ///
    /// Fails when the XML cannot be written.
    pub fn domain_xml(&self) -> Result<String, LibvirtError> {
        let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
        self.write_domain(&mut writer)
            .map_err(|_| LibvirtError::DomainXmlWrite)?;
        String::from_utf8(writer.into_inner()).map_err(|_| LibvirtError::DomainXmlWrite)
    }

    fn write_domain(&self, w: &mut Writer<Vec<u8>>) -> io::Result<()> {
        w.create_element("domain")
            .with_attribute(("type", "kvm"))
            .write_inner_content(|w| {
                text(w, "name", &self.name)?;
                w.create_element("memory")
                    .with_attribute(("unit", "MiB"))
                    .write_text_content(BytesText::new(&self.memory_mib.to_string()))?;
                text(w, "vcpu", &self.vcpus.to_string())?;
                w.create_element("memoryBacking").write_inner_content(|w| {
                    w.create_element("source")
                        .with_attribute(("type", "memfd"))
                        .write_empty()?;
                    w.create_element("access")
                        .with_attribute(("mode", "shared"))
                        .write_empty()?;
                    Ok::<(), io::Error>(())
                })?;
                w.create_element("os").write_inner_content(|w| {
                    w.create_element("type")
                        .with_attributes([("arch", "x86_64"), ("machine", "q35")])
                        .write_text_content(BytesText::new("hvm"))?;
                    text(w, "kernel", &self.kernel)?;
                    text(w, "initrd", &self.initrd)?;
                    text(w, "cmdline", KERNEL_CMDLINE)?;
                    Ok::<(), io::Error>(())
                })?;
                w.create_element("features").write_inner_content(|w| {
                    w.create_element("acpi").write_empty()?;
                    Ok::<(), io::Error>(())
                })?;
                w.create_element("cpu")
                    .with_attribute(("mode", "host-passthrough"))
                    .write_empty()?;
                text(w, "on_poweroff", "destroy")?;
                text(w, "on_reboot", "destroy")?;
                text(w, "on_crash", "destroy")?;
                w.create_element("devices")
                    .write_inner_content(|w| self.write_devices(w))?;
                Ok::<(), io::Error>(())
            })?;
        Ok(())
    }

    fn write_devices(&self, w: &mut Writer<Vec<u8>>) -> io::Result<()> {
        share(w, &self.rootfs_dir, ROOTFS_TAG)?;
        share(w, &self.node_dir, NODE_TAG)?;
        w.create_element("disk")
            .with_attributes([("type", "file"), ("device", "disk")])
            .write_inner_content(|w| {
                w.create_element("driver")
                    .with_attributes([("name", "qemu"), ("type", "raw")])
                    .write_empty()?;
                w.create_element("source")
                    .with_attribute(("file", self.disk.as_str()))
                    .write_empty()?;
                w.create_element("target")
                    .with_attributes([("dev", "vda"), ("bus", "virtio")])
                    .write_empty()?;
                Ok::<(), io::Error>(())
            })?;
        w.create_element("interface")
            .with_attribute(("type", "network"))
            .write_inner_content(|w| {
                w.create_element("mac")
                    .with_attribute(("address", self.management_mac.as_str()))
                    .write_empty()?;
                w.create_element("source")
                    .with_attribute(("network", self.management_network.as_str()))
                    .write_empty()?;
                virtio_model(w)
            })?;
        for nic in &self.nics {
            w.create_element("interface")
                .with_attribute(("type", "ethernet"))
                .write_inner_content(|w| {
                    w.create_element("mac")
                        .with_attribute(("address", nic.mac.as_str()))
                        .write_empty()?;
                    w.create_element("target")
                        .with_attribute(("dev", nic.tap.as_str()))
                        .write_empty()?;
                    virtio_model(w)?;
                    w.create_element("link")
                        .with_attribute(("state", if nic.up { "up" } else { "down" }))
                        .write_empty()?;
                    Ok::<(), io::Error>(())
                })?;
        }
        w.create_element("serial")
            .with_attribute(("type", "unix"))
            .write_inner_content(|w| {
                w.create_element("source")
                    .with_attributes([("mode", "bind"), ("path", self.serial_socket.as_str())])
                    .write_empty()?;
                w.create_element("log")
                    .with_attributes([("file", self.serial_log.as_str()), ("append", "off")])
                    .write_empty()?;
                w.create_element("target")
                    .with_attributes([("type", "isa-serial"), ("port", "0")])
                    .write_empty()?;
                Ok::<(), io::Error>(())
            })?;
        w.create_element("console")
            .with_attribute(("type", "unix"))
            .write_inner_content(|w| {
                w.create_element("source")
                    .with_attributes([("mode", "bind"), ("path", self.serial_socket.as_str())])
                    .write_empty()?;
                w.create_element("target")
                    .with_attributes([("type", "serial"), ("port", "0")])
                    .write_empty()?;
                Ok::<(), io::Error>(())
            })?;
        w.create_element("video").write_inner_content(|w| {
            w.create_element("model")
                .with_attribute(("type", "none"))
                .write_empty()?;
            Ok::<(), io::Error>(())
        })?;
        w.create_element("memballoon")
            .with_attribute(("model", "none"))
            .write_empty()?;
        Ok(())
    }
}

/// Write `<name>value</name>`.
fn text(w: &mut Writer<Vec<u8>>, name: &str, value: &str) -> io::Result<()> {
    w.create_element(name)
        .write_text_content(BytesText::new(value))?;
    Ok(())
}

/// Write a virtio NIC model.
fn virtio_model(w: &mut Writer<Vec<u8>>) -> io::Result<()> {
    w.create_element("model")
        .with_attribute(("type", "virtio"))
        .write_empty()?;
    Ok(())
}

/// Write a virtio-fs share of host directory `dir` under `tag`, with extended attributes, so
/// file capabilities reach the guest.
fn share(w: &mut Writer<Vec<u8>>, dir: &str, tag: &str) -> io::Result<()> {
    w.create_element("filesystem")
        .with_attributes([("type", "mount"), ("accessmode", "passthrough")])
        .write_inner_content(|w| {
            w.create_element("driver")
                .with_attribute(("type", "virtiofs"))
                .write_empty()?;
            w.create_element("binary")
                .with_attribute(("xattr", "on"))
                .write_empty()?;
            w.create_element("source")
                .with_attribute(("dir", dir))
                .write_empty()?;
            w.create_element("target")
                .with_attribute(("dir", tag))
                .write_empty()?;
            Ok::<(), io::Error>(())
        })?;
    Ok(())
}

/// Define container VM `vm` as a persistent domain and start it. A domain that fails to start
/// stays defined; destroy finds it by name and removes it.
///
/// # Errors
///
/// Fails when the XML cannot be written, or libvirt cannot define or start the domain.
#[instrument(level = "debug", skip(conn, vm), fields(domain = %vm.name))]
pub fn create_container_vm(conn: &Connect, vm: &ContainerVm) -> Result<()> {
    let xml = vm.domain_xml()?;
    let domain = Domain::define_xml(conn, &xml)
        .with_context(|| format!("Cannot define container VM {}", vm.name))?;
    domain
        .create()
        .with_context(|| format!("Cannot start container VM {}", vm.name))?;
    Ok(())
}

/// The XML of a sparse raw volume `name` of `capacity_bytes`.
fn raw_volume_xml(name: &str, capacity_bytes: u64) -> Result<String, LibvirtError> {
    let mut writer = Writer::new(Vec::new());
    writer
        .create_element("volume")
        .with_attribute(("type", "file"))
        .write_inner_content(|w| {
            text(w, "name", name)?;
            w.create_element("capacity")
                .with_attribute(("unit", "bytes"))
                .write_text_content(BytesText::new(&capacity_bytes.to_string()))?;
            text(w, "allocation", "0")?;
            w.create_element("target").write_inner_content(|w| {
                w.create_element("format")
                    .with_attribute(("type", "raw"))
                    .write_empty()?;
                Ok::<(), io::Error>(())
            })?;
            Ok::<(), io::Error>(())
        })
        .map_err(|_| LibvirtError::DomainXmlWrite)?;
    String::from_utf8(writer.into_inner()).map_err(|_| LibvirtError::DomainXmlWrite)
}

/// The path of volume `name` in the Sherpa storage pool, created as a sparse raw disk of
/// `capacity_bytes` when the pool has no such volume. A volume that is there is kept with what
/// it holds, so a node's changes outlive its domain.
///
/// # Errors
///
/// Fails when the pool cannot be found or refreshed, or libvirt cannot create the volume or
/// read its path.
#[instrument(level = "debug", skip(conn))]
pub fn ensure_raw_volume(conn: &Connect, name: &str, capacity_bytes: u64) -> Result<String> {
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)
        .with_context(|| format!("Cannot find storage pool '{SHERPA_STORAGE_POOL}'"))?;
    refresh_pool(&pool)?;
    let volume = match StorageVol::lookup_by_name(&pool, name) {
        Ok(volume) => volume,
        Err(error) if error.code() == ErrorNumber::NoStorageVolume => {
            StorageVol::create_xml(&pool, &raw_volume_xml(name, capacity_bytes)?, 0)
                .with_context(|| format!("Cannot create volume '{name}'"))?
        }
        Err(error) => return Err(error).with_context(|| format!("Cannot look up volume '{name}'")),
    };
    volume
        .get_path()
        .with_context(|| format!("Cannot read the path of volume '{name}'"))
}

/// Delete volume `name` from the Sherpa storage pool; one already gone counts as deleted.
///
/// # Errors
///
/// Fails when the pool cannot be found or refreshed, or libvirt cannot delete the volume.
#[instrument(level = "debug", skip(conn))]
pub fn delete_volume_if_present(conn: &Connect, name: &str) -> Result<()> {
    let pool = StoragePool::lookup_by_name(conn, SHERPA_STORAGE_POOL)
        .with_context(|| format!("Cannot find storage pool '{SHERPA_STORAGE_POOL}'"))?;
    refresh_pool(&pool)?;
    match StorageVol::lookup_by_name(&pool, name) {
        Ok(volume) => volume
            .delete(0)
            .with_context(|| format!("Cannot delete volume '{name}'")),
        Err(error) if error.code() == ErrorNumber::NoStorageVolume => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Cannot look up volume '{name}'")),
    }
}

/// Whether domain `name` is running, or `None` when it does not exist.
///
/// # Errors
///
/// Fails when libvirt cannot look the domain up or read its state.
#[instrument(level = "debug", skip(conn))]
pub fn domain_state(conn: &Connect, name: &str) -> Result<Option<bool>> {
    let domain = match Domain::lookup_by_name(conn, name) {
        Ok(domain) => domain,
        Err(error) if error.code() == ErrorNumber::NoDomain => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("Cannot look up domain {name}")),
    };
    let (state, _) = domain
        .get_state()
        .with_context(|| format!("Cannot read the state of domain {name}"))?;
    Ok(Some(state == VIR_DOMAIN_RUNNING))
}

/// Stop domain `name` if it runs and undefine it. A domain already gone counts as removed, so
/// a retried teardown succeeds.
///
/// # Errors
///
/// Fails when libvirt cannot look the domain up, stop it or undefine it.
#[instrument(level = "debug", skip(conn))]
pub fn remove_domain(conn: &Connect, name: &str) -> Result<()> {
    let domain = match Domain::lookup_by_name(conn, name) {
        Ok(domain) => domain,
        Err(error) if error.code() == ErrorNumber::NoDomain => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("Cannot look up domain {name}")),
    };
    if domain
        .is_active()
        .with_context(|| format!("Cannot read whether domain {name} runs"))?
    {
        domain
            .destroy()
            .with_context(|| format!("Cannot stop domain {name}"))?;
    }
    domain
        .undefine()
        .with_context(|| format!("Cannot undefine domain {name}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm() -> ContainerVm {
        ContainerVm {
            name: "srl1-abc123".to_string(),
            memory_mib: 4096,
            vcpus: 2,
            kernel: "/opt/sherpa/vmboot/vmlinuz".to_string(),
            initrd: "/opt/sherpa/vmboot/initramfs.img".to_string(),
            rootfs_dir: "/opt/sherpa/vmroot-ro/0096fe3e".to_string(),
            node_dir: "/opt/sherpa/labs/abc123/srl1/vm".to_string(),
            disk: "/opt/sherpa/libvirt/images/srl1-abc123-disk.img".to_string(),
            management_network: "sherpa-management-abc123".to_string(),
            management_mac: "52:54:00:00:00:01".to_string(),
            nics: vec![
                ContainerVmNic {
                    tap: "tpa1-abc123".to_string(),
                    mac: "52:54:00:00:00:02".to_string(),
                    up: true,
                },
                ContainerVmNic {
                    tap: "tpb2-abc123".to_string(),
                    mac: "52:54:00:00:00:03".to_string(),
                    up: false,
                },
            ],
            serial_socket: "/opt/sherpa/run/console/abc123/1.serial".to_string(),
            serial_log: "/var/log/libvirt/qemu/srl1-abc123-serial.log".to_string(),
        }
    }

    #[test]
    fn container_vm_001_the_domain_boots_the_kernel_and_initramfs_directly() {
        let xml = vm().domain_xml().unwrap();
        assert!(xml.contains("<kernel>/opt/sherpa/vmboot/vmlinuz</kernel>"));
        assert!(xml.contains("<initrd>/opt/sherpa/vmboot/initramfs.img</initrd>"));
        assert!(xml.contains("<cmdline>console=ttyS0 rdinit=/init panic=-1 reboot=t</cmdline>"));
        assert!(xml.contains(r#"machine="q35""#));
        assert!(xml.contains(r#"<model type="none"/>"#));
        assert!(xml.contains(r#"<memballoon model="none"/>"#));
    }

    #[test]
    fn container_vm_002_rootfs_and_node_files_are_virtiofs_shares_with_xattrs_on_shared_memory() {
        let xml = vm().domain_xml().unwrap();
        assert!(xml.contains(r#"<source type="memfd"/>"#));
        assert!(xml.contains(r#"<access mode="shared"/>"#));
        assert_eq!(xml.matches(r#"<driver type="virtiofs"/>"#).count(), 2);
        assert_eq!(xml.matches(r#"<binary xattr="on"/>"#).count(), 2);
        assert!(xml.contains(r#"<source dir="/opt/sherpa/vmroot-ro/0096fe3e"/>"#));
        assert!(xml.contains(r#"<target dir="rootfs"/>"#));
        assert!(xml.contains(r#"<target dir="node"/>"#));
    }

    #[test]
    fn container_vm_003_each_p2p_interface_is_a_tap_named_like_the_container_host_veth() {
        let xml = vm().domain_xml().unwrap();
        assert!(xml.contains(r#"<target dev="tpa1-abc123"/>"#));
        assert!(xml.contains(r#"<target dev="tpb2-abc123"/>"#));
        assert!(xml.contains(r#"<source network="sherpa-management-abc123"/>"#));
        assert_eq!(xml.matches(r#"<link state="down"/>"#).count(), 1);
    }

    #[test]
    fn container_vm_004_the_serial_console_is_on_the_nodes_console_socket() {
        let xml = vm().domain_xml().unwrap();
        assert!(
            xml.contains(r#"<source mode="bind" path="/opt/sherpa/run/console/abc123/1.serial"/>"#)
        );
        assert!(xml.contains(
            r#"<log file="/var/log/libvirt/qemu/srl1-abc123-serial.log" append="off"/>"#
        ));
    }

    #[test]
    fn container_vm_005_a_guest_that_stops_or_reboots_stops_its_domain_like_an_exited_container() {
        let xml = vm().domain_xml().unwrap();
        assert!(xml.contains("<on_reboot>destroy</on_reboot>"));
        assert!(xml.contains("<on_crash>destroy</on_crash>"));
    }

    #[test]
    fn container_vm_017_the_nodes_raw_disk_is_a_virtio_disk() {
        let xml = vm().domain_xml().unwrap();
        assert!(xml.contains(r#"<driver name="qemu" type="raw"/>"#));
        assert!(
            xml.contains(r#"<source file="/opt/sherpa/libvirt/images/srl1-abc123-disk.img"/>"#)
        );
        assert!(xml.contains(r#"<target dev="vda" bus="virtio"/>"#));
    }

    #[test]
    fn container_vm_018_the_node_disk_volume_is_sparse_raw() {
        let xml = raw_volume_xml("srl1-abc123-disk.img", 1 << 33).unwrap();
        assert!(xml.contains("<name>srl1-abc123-disk.img</name>"));
        assert!(xml.contains(r#"<capacity unit="bytes">8589934592</capacity>"#));
        assert!(xml.contains("<allocation>0</allocation>"));
        assert!(xml.contains(r#"<format type="raw"/>"#));
    }

    #[test]
    fn container_vm_006_values_are_escaped() {
        let mut vm = vm();
        vm.name = "a<b".to_string();
        assert!(vm.domain_xml().unwrap().contains("<name>a&lt;b</name>"));
    }
}
