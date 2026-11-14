//! The link state of a running domain's interface: what the guest sees as carrier.
//!
//! The change is made to the live domain only, as `virsh domif-setlink` makes it: the
//! interface's live definition is taken from the domain's XML, its `link` element is set, and
//! the result is handed to libvirt as a device update. The persistent definition is not
//! changed, so a domain that is started again has its links up.

use anyhow::{Context, Result, bail};
use roxmltree::{Document, Node};
use tracing::instrument;
use virt::connect::Connect;
use virt::domain::Domain;
use virt::error::ErrorNumber;
use virt::sys::VIR_DOMAIN_AFFECT_LIVE;

/// Whether the interface on host tap `tap` of running domain `domain` has its link up. `None`
/// when the domain does not exist, is not running, or has no interface on `tap`.
///
/// # Errors
///
/// Fails when libvirt cannot look the domain up or read its live XML, or the XML cannot be
/// parsed.
#[instrument(level = "debug", skip(conn))]
pub fn interface_link_up(conn: &Connect, domain: &str, tap: &str) -> Result<Option<bool>> {
    let Some(xml) = live_xml(conn, domain)? else {
        return Ok(None);
    };
    link_up_in(&xml, tap)
}

/// Set the link of the interface on host tap `tap` of running domain `domain` up or down, in
/// the live domain only.
///
/// # Errors
///
/// Fails when the domain is not running, has no interface on `tap`, or libvirt refuses the
/// change.
#[instrument(level = "debug", skip(conn))]
pub fn set_interface_link(conn: &Connect, domain: &str, tap: &str, up: bool) -> Result<()> {
    let Some(xml) = live_xml(conn, domain)? else {
        bail!("Domain '{domain}' is not running");
    };
    let Some(interface) = interface_with_link(&xml, tap, up)? else {
        bail!("Domain '{domain}' has no interface on {tap}");
    };
    let handle = Domain::lookup_by_name(conn, domain)
        .with_context(|| format!("Cannot find domain '{domain}'"))?;
    handle
        .update_device_flags(&interface, VIR_DOMAIN_AFFECT_LIVE)
        .with_context(|| format!("Cannot set the link of {tap} on domain '{domain}'"))?;
    Ok(())
}

/// The live XML of domain `domain`, or `None` when it does not exist or is not running.
fn live_xml(conn: &Connect, domain: &str) -> Result<Option<String>> {
    let handle = match Domain::lookup_by_name(conn, domain) {
        Ok(handle) => handle,
        Err(error) if error.code() == ErrorNumber::NoDomain => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot find domain '{domain}'"));
        }
    };
    let active = handle
        .is_active()
        .with_context(|| format!("Cannot read whether domain '{domain}' is running"))?;
    if !active {
        return Ok(None);
    }
    handle
        .get_xml_desc(0)
        .map(Some)
        .with_context(|| format!("Cannot read the XML of domain '{domain}'"))
}

/// The `interface` element of `doc` whose `target` is host device `tap`.
fn find_interface<'a, 'input>(doc: &'a Document<'input>, tap: &str) -> Option<Node<'a, 'input>> {
    doc.descendants()
        .filter(|node| node.has_tag_name("interface"))
        .find(|interface| {
            interface
                .children()
                .any(|child| child.has_tag_name("target") && child.attribute("dev") == Some(tap))
        })
}

/// Whether the interface on `tap` in domain XML `xml` has its link up: it is, unless its
/// `link` element says `down`. `None` when there is no interface on `tap`.
fn link_up_in(xml: &str, tap: &str) -> Result<Option<bool>> {
    let doc = Document::parse(xml).context("Cannot parse the domain XML")?;
    Ok(find_interface(&doc, tap).map(|interface| {
        interface
            .children()
            .find(|child| child.has_tag_name("link"))
            .and_then(|link| link.attribute("state"))
            != Some("down")
    }))
}

/// The XML of the interface on `tap` in domain XML `xml`, with its link state set to `up`.
/// `None` when there is no interface on `tap`.
fn interface_with_link(xml: &str, tap: &str, up: bool) -> Result<Option<String>> {
    let doc = Document::parse(xml).context("Cannot parse the domain XML")?;
    let Some(interface) = find_interface(&doc, tap) else {
        return Ok(None);
    };
    let state = if up { "up" } else { "down" };
    let link = format!("<link state='{state}'/>");
    let range = interface.range();
    let element = &xml[range.clone()];
    if let Some(old) = interface
        .children()
        .find(|child| child.has_tag_name("link"))
    {
        let old_range = old.range();
        let start = old_range.start - range.start;
        let end = old_range.end - range.start;
        return Ok(Some(format!(
            "{}{link}{}",
            &element[..start],
            &element[end..]
        )));
    }
    let Some(body) = element.strip_suffix("</interface>") else {
        bail!("The interface on {tap} has no closing tag");
    };
    Ok(Some(format!("{body}{link}</interface>")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN: &str = "<domain type='kvm'><name>r1-lab</name><devices>\
        <interface type='bridge'><mac address='52:54:00:00:00:01'/><source bridge='br0'/>\
        <target dev='vnet0'/><model type='virtio'/></interface>\
        <interface type='ethernet'><mac address='52:54:00:00:00:02'/>\
        <target dev='tpa0' managed='yes'/><model type='virtio'/>\
        <alias name='net1'/><address type='pci' domain='0x0000' bus='0x01' slot='0x00' function='0x0'/>\
        </interface>\
        <interface type='ethernet'><mac address='52:54:00:00:00:03'/>\
        <target dev='tpa1' managed='yes'/><model type='virtio'/><link state='down'/>\
        </interface></devices></domain>";

    #[test]
    fn an_interface_has_its_link_up_unless_its_link_says_down() {
        assert_eq!(link_up_in(DOMAIN, "tpa0").unwrap(), Some(true));
        assert_eq!(link_up_in(DOMAIN, "tpa1").unwrap(), Some(false));
        assert_eq!(link_up_in(DOMAIN, "tpa9").unwrap(), None);
    }

    #[test]
    fn setting_the_link_keeps_the_rest_of_the_interface() {
        let down = interface_with_link(DOMAIN, "tpa0", false).unwrap().unwrap();
        assert_eq!(
            down,
            "<interface type='ethernet'><mac address='52:54:00:00:00:02'/>\
             <target dev='tpa0' managed='yes'/><model type='virtio'/>\
             <alias name='net1'/><address type='pci' domain='0x0000' bus='0x01' slot='0x00' function='0x0'/>\
             <link state='down'/></interface>"
        );
        assert_eq!(link_up_in(&down, "tpa0").unwrap(), Some(false));
    }

    #[test]
    fn setting_the_link_replaces_a_link_the_interface_has() {
        let up = interface_with_link(DOMAIN, "tpa1", true).unwrap().unwrap();
        assert_eq!(
            up,
            "<interface type='ethernet'><mac address='52:54:00:00:00:03'/>\
             <target dev='tpa1' managed='yes'/><model type='virtio'/><link state='up'/>\
             </interface>"
        );
    }

    #[test]
    fn a_tap_no_interface_is_on_has_no_link_to_set() {
        assert_eq!(interface_with_link(DOMAIN, "tpa9", false).unwrap(), None);
        assert!(link_up_in("<domain", "tpa0").is_err());
    }
}
