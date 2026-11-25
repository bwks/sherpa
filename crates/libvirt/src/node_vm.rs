use std::io;

use anyhow::{Context, Result};
use quick_xml::Writer;
use quick_xml::events::{BytesText, Event};
use quick_xml::reader::Reader;
use tracing::instrument;
use virt::connect::Connect;
use virt::domain::Domain;
use virt::error::ErrorNumber;
use virt::storage_pool::StoragePool;
use virt::storage_vol::StorageVol;

use shared::konst::{MTU_JUMBO_NET, SHERPA_STORAGE_POOL};

use crate::error::LibvirtError;
use crate::storage::refresh_pool;

/// A Flatcar VM that runs one container node on its own Docker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeVm {
    pub name: String,
    pub memory_mib: u64,
    pub vcpus: u8,
    pub disk: String,
    pub config_iso: String,
    pub management_network: String,
    pub management_mac: String,
    pub nics: Vec<NodeVmNic>,
    pub serial_socket: String,
}

/// A node VM's data interface: a virtio NIC whose host side is the tap `tap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeVmNic {
    pub tap: String,
    pub mac: String,
    pub up: bool,
}

impl NodeVm {
    /// The domain XML of the node VM.
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
                w.create_element("os").write_inner_content(|w| {
                    w.create_element("type")
                        .with_attributes([("arch", "x86_64"), ("machine", "q35")])
                        .write_text_content(BytesText::new("hvm"))?;
                    w.create_element("boot")
                        .with_attribute(("dev", "hd"))
                        .write_empty()?;
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
                text(w, "on_reboot", "restart")?;
                text(w, "on_crash", "destroy")?;
                w.create_element("devices")
                    .write_inner_content(|w| self.write_devices(w))?;
                Ok::<(), io::Error>(())
            })?;
        Ok(())
    }

    fn write_devices(&self, w: &mut Writer<Vec<u8>>) -> io::Result<()> {
        self.write_disks(w)?;
        self.write_interfaces(w)?;
        self.write_consoles(w)?;
        w.create_element("rng")
            .with_attribute(("model", "virtio"))
            .write_inner_content(|w| {
                w.create_element("backend")
                    .with_attribute(("model", "random"))
                    .write_text_content(BytesText::new("/dev/urandom"))?;
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

    fn write_disks(&self, w: &mut Writer<Vec<u8>>) -> io::Result<()> {
        w.create_element("disk")
            .with_attributes([("type", "file"), ("device", "disk")])
            .write_inner_content(|w| {
                w.create_element("driver")
                    .with_attributes([("name", "qemu"), ("type", "qcow2")])
                    .write_empty()?;
                w.create_element("source")
                    .with_attribute(("file", self.disk.as_str()))
                    .write_empty()?;
                w.create_element("target")
                    .with_attributes([("dev", "vda"), ("bus", "virtio")])
                    .write_empty()?;
                Ok::<(), io::Error>(())
            })?;
        w.create_element("disk")
            .with_attributes([("type", "file"), ("device", "cdrom")])
            .write_inner_content(|w| {
                w.create_element("driver")
                    .with_attributes([("name", "qemu"), ("type", "raw")])
                    .write_empty()?;
                w.create_element("source")
                    .with_attribute(("file", self.config_iso.as_str()))
                    .write_empty()?;
                w.create_element("target")
                    .with_attributes([("dev", "sda"), ("bus", "sata")])
                    .write_empty()?;
                w.create_element("readonly").write_empty()?;
                Ok::<(), io::Error>(())
            })?;
        Ok(())
    }

    fn write_interfaces(&self, w: &mut Writer<Vec<u8>>) -> io::Result<()> {
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
                    w.create_element("mtu")
                        .with_attribute(("size", MTU_JUMBO_NET.to_string().as_str()))
                        .write_empty()?;
                    w.create_element("mac")
                        .with_attribute(("address", nic.mac.as_str()))
                        .write_empty()?;
                    w.create_element("target")
                        .with_attribute(("dev", nic.tap.as_str()))
                        .write_empty()?;
                    w.create_element("script")
                        .with_attribute(("path", ""))
                        .write_empty()?;
                    virtio_model(w)?;
                    w.create_element("link")
                        .with_attribute(("state", if nic.up { "up" } else { "down" }))
                        .write_empty()?;
                    Ok::<(), io::Error>(())
                })?;
        }
        Ok(())
    }

    fn write_consoles(&self, w: &mut Writer<Vec<u8>>) -> io::Result<()> {
        w.create_element("vsock")
            .with_attribute(("model", "virtio"))
            .write_inner_content(|w| {
                w.create_element("cid")
                    .with_attribute(("auto", "yes"))
                    .write_empty()?;
                Ok::<(), io::Error>(())
            })?;
        w.create_element("serial")
            .with_attribute(("type", "unix"))
            .write_inner_content(|w| {
                w.create_element("source")
                    .with_attributes([("mode", "bind"), ("path", self.serial_socket.as_str())])
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

/// The vsock context ID libvirt gave a running domain, from its live XML, or `None` when the
/// XML has none.
///
/// # Errors
///
/// Fails when the XML cannot be parsed or the ID is not a number.
pub fn vsock_cid(domain_xml: &str) -> Result<Option<u32>, LibvirtError> {
    let mut reader = Reader::from_str(domain_xml);
    let mut in_vsock = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) if element.name().as_ref() == "vsock" => in_vsock = true,
            Ok(Event::End(element)) if element.name().as_ref() == "vsock" => in_vsock = false,
            Ok(Event::Empty(element) | Event::Start(element))
                if in_vsock && element.name().as_ref() == "cid" =>
            {
                let address = element
                    .try_get_attribute("address")
                    .map_err(|_| LibvirtError::DomainXmlInvalid)?;
                return address
                    .map(|address| {
                        address
                            .value
                            .parse()
                            .map_err(|_| LibvirtError::DomainXmlInvalid)
                    })
                    .transpose();
            }
            Ok(Event::Eof) => return Ok(None),
            Ok(_) => {}
            Err(_) => {
                return Err(LibvirtError::DomainXmlInvalidAt {
                    position: reader.buffer_position(),
                });
            }
        }
    }
}

/// Define node VM `vm` as a persistent domain and start it. A domain that libvirt cannot start
/// is undefined again, so a retry can define it under the same name.
///
/// # Errors
///
/// Fails when the XML cannot be written, or libvirt cannot define or start the domain.
#[instrument(level = "debug", skip(conn, vm), fields(domain = %vm.name))]
pub fn create_node_vm(conn: &Connect, vm: &NodeVm) -> Result<()> {
    let xml = vm.domain_xml()?;
    let domain = Domain::define_xml(conn, &xml)
        .with_context(|| format!("Cannot define node VM {}", vm.name))?;
    if let Err(error) = domain.create() {
        if let Err(undefine) = domain.undefine() {
            tracing::warn!(domain = %vm.name, error = %undefine, "Cannot undefine a node VM that did not start");
        }
        return Err(error).with_context(|| format!("Cannot start node VM {}", vm.name));
    }
    Ok(())
}

/// The vsock context ID of running domain `name`, or `None` when it is not defined, not
/// running or has no vsock device.
///
/// # Errors
///
/// Fails when libvirt cannot look the domain up or read its XML, or the XML cannot be parsed.
#[instrument(level = "debug", skip(conn))]
pub fn node_vm_cid(conn: &Connect, name: &str) -> Result<Option<u32>> {
    let domain = match Domain::lookup_by_name(conn, name) {
        Ok(domain) => domain,
        Err(error) if error.code() == ErrorNumber::NoDomain => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("Cannot look up domain {name}")),
    };
    if !domain
        .is_active()
        .with_context(|| format!("Cannot read whether domain {name} runs"))?
    {
        return Ok(None);
    }
    let xml = domain
        .get_xml_desc(0)
        .with_context(|| format!("Cannot read the XML of domain {name}"))?;
    Ok(vsock_cid(&xml)?)
}

/// Stop domain `name` if it runs and undefine it. A domain already gone counts as removed, so
/// a retried teardown succeeds.
///
/// # Errors
///
/// Fails when libvirt cannot look the domain up, stop it or undefine it.
#[instrument(level = "debug", skip(conn))]
pub fn remove_node_vm(conn: &Connect, name: &str) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn vm() -> NodeVm {
        NodeVm {
            name: "ceos1-abc12345".to_string(),
            memory_mib: 6144,
            vcpus: 2,
            disk: "/opt/sherpa/libvirt/images/ceos1-abc12345-hdd.qcow2".to_string(),
            config_iso: "/opt/sherpa/libvirt/images/ceos1-abc12345.iso".to_string(),
            management_network: "sherpa-management-abc12345".to_string(),
            management_mac: "52:54:00:00:00:01".to_string(),
            nics: vec![
                NodeVmNic {
                    tap: "tpa1-abc12345".to_string(),
                    mac: "52:54:00:00:00:02".to_string(),
                    up: true,
                },
                NodeVmNic {
                    tap: "tpb2-abc12345".to_string(),
                    mac: "52:54:00:00:00:03".to_string(),
                    up: false,
                },
            ],
            serial_socket: "/opt/sherpa/run/console/abc12345/1.serial".to_string(),
        }
    }

    #[test]
    fn node_vm_001_the_domain_boots_its_overlay_disk_with_the_ignition_config_disk() {
        let xml = vm().domain_xml().unwrap();
        assert!(xml.contains(r#"<driver name="qemu" type="qcow2"/>"#));
        assert!(
            xml.contains(r#"<source file="/opt/sherpa/libvirt/images/ceos1-abc12345-hdd.qcow2"/>"#)
        );
        assert!(xml.contains(r#"<disk type="file" device="cdrom">"#));
        assert!(xml.contains(r#"<source file="/opt/sherpa/libvirt/images/ceos1-abc12345.iso"/>"#));
        assert!(xml.contains("<readonly/>"));
        assert!(xml.contains(r#"<boot dev="hd"/>"#));
        assert!(xml.contains(r#"machine="q35""#));
    }

    #[test]
    fn node_vm_002_each_data_interface_is_a_tap_named_like_its_link_end() {
        let xml = vm().domain_xml().unwrap();
        assert!(xml.contains(r#"<target dev="tpa1-abc12345"/>"#));
        assert!(xml.contains(r#"<target dev="tpb2-abc12345"/>"#));
        assert!(xml.contains(r#"<mac address="52:54:00:00:00:03"/>"#));
        assert_eq!(xml.matches(r#"<link state="down"/>"#).count(), 1);
        assert_eq!(xml.matches(r#"<link state="up"/>"#).count(), 1);
        assert!(xml.contains(r#"<source network="sherpa-management-abc12345"/>"#));
    }

    #[test]
    fn node_vm_003_the_vm_has_a_vsock_and_its_serial_console_on_the_nodes_socket() {
        let xml = vm().domain_xml().unwrap();
        assert!(xml.contains(r#"<vsock model="virtio">"#));
        assert!(xml.contains(r#"<cid auto="yes"/>"#));
        assert!(
            xml.contains(
                r#"<source mode="bind" path="/opt/sherpa/run/console/abc12345/1.serial"/>"#
            )
        );
        assert!(xml.contains(r#"<model type="none"/>"#));
        assert!(xml.contains(r#"<memballoon model="none"/>"#));
    }

    #[test]
    fn node_vm_004_the_vsock_id_is_read_from_the_live_xml() {
        let live = r#"<domain><devices><vsock model="virtio"><cid auto="yes" address="7"/></vsock></devices></domain>"#;
        assert_eq!(vsock_cid(live).unwrap(), Some(7));
        let defined = r#"<domain><devices><vsock model="virtio"><cid auto="yes"/></vsock></devices></domain>"#;
        assert_eq!(vsock_cid(defined).unwrap(), None);
        assert_eq!(
            vsock_cid("<domain><cid address='3'/></domain>").unwrap(),
            None
        );
        assert!(vsock_cid(r#"<domain><vsock><cid address="x"/></vsock></domain>"#).is_err());
    }

    #[test]
    fn node_vm_005_values_are_escaped() {
        let mut vm = vm();
        vm.name = "a<b".to_string();
        assert!(vm.domain_xml().unwrap().contains("<name>a&lt;b</name>"));
    }
}
