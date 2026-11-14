//! The link state of a running domain's interface: what the guest sees as carrier.
//!
//! The change is made to the live domain only, as `virsh domif-setlink` makes it: the
//! interface's live definition is taken from the domain's XML, its `link` element is set, and
//! the result is handed to libvirt as a device update. The persistent definition is not
//! changed, so a domain that is started again has its links up.

use std::borrow::Cow;
use std::mem;

use anyhow::{Context, Result, bail};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer, XmlVersion};
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

/// The events of the `interface` element of domain XML `xml` whose own `target` is host
/// device `tap`, from its start tag to its end tag, each with its depth below the interface:
/// 0 for the interface's own tags, 1 for its children. `None` when there is no interface on
/// `tap`. The whole document is read, so XML that is not well formed is refused wherever it
/// fails.
fn find_interface<'a>(xml: &'a str, tap: &str) -> Result<Option<Vec<(usize, Event<'a>)>>> {
    let mut reader = Reader::from_str(xml);
    // The elements open before the event read, and the depth of the interface being read.
    let mut open = 0usize;
    let mut interface_depth = None;
    let mut events = Vec::new();
    let mut on_tap = false;
    let mut found = None;
    loop {
        let event = reader.read_event().with_context(|| {
            format!(
                "Cannot parse the domain XML at byte {}",
                reader.error_position()
            )
        })?;
        // The depth of the element the event opens, closes or sits in.
        let depth = match &event {
            Event::Start(_) => {
                open += 1;
                open - 1
            }
            Event::End(_) => {
                open = open
                    .checked_sub(1)
                    .context("Cannot parse the domain XML: an end tag closes nothing")?;
                open
            }
            Event::Eof if open > 0 => {
                bail!("Cannot parse the domain XML: it ends inside an element")
            }
            Event::Eof => break,
            _ => open,
        };
        // Attributes are only parsed when asked for; reading them all refuses duplicates.
        if let Event::Start(element) | Event::Empty(element) = &event {
            for attribute in element.attributes() {
                attribute.context("Cannot parse the domain XML")?;
            }
        }
        if found.is_some() {
            continue;
        }
        if let Some(start) = interface_depth {
            let below = depth - start;
            if below == 1 && attribute(&event, "target", "dev")?.as_deref() == Some(tap) {
                on_tap = true;
            }
            let closes = below == 0 && matches!(event, Event::End(_));
            events.push((below, event));
            if closes {
                if on_tap {
                    found = Some(mem::take(&mut events));
                } else {
                    events.clear();
                }
                interface_depth = None;
                on_tap = false;
            }
        } else if let Event::Start(element) = &event
            && element.name().as_ref() == "interface"
        {
            interface_depth = Some(depth);
            events.push((0, event));
        }
    }
    Ok(found)
}

/// The value of attribute `name` when `event` opens an element named `tag`; `None` for any
/// other event, or when the element does not have the attribute.
fn attribute<'a>(event: &'a Event, tag: &str, name: &str) -> Result<Option<Cow<'a, str>>> {
    let (Event::Start(element) | Event::Empty(element)) = event else {
        return Ok(None);
    };
    if element.name().as_ref() != tag {
        return Ok(None);
    }
    for attribute in element.attributes() {
        let attribute = attribute.context("Cannot parse the domain XML")?;
        if attribute.key.as_ref() == name {
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .context("Cannot parse the domain XML")?;
            return Ok(Some(value));
        }
    }
    Ok(None)
}

/// Whether `event` opens an element named `tag`.
fn opens(event: &Event, tag: &str) -> bool {
    matches!(event, Event::Start(element) | Event::Empty(element) if element.name().as_ref() == tag)
}

/// Whether the interface on `tap` in domain XML `xml` has its link up: it is, unless its
/// `link` element says `down`. `None` when there is no interface on `tap`.
fn link_up_in(xml: &str, tap: &str) -> Result<Option<bool>> {
    let Some(events) = find_interface(xml, tap)? else {
        return Ok(None);
    };
    for (depth, event) in &events {
        if *depth == 1 && opens(event, "link") {
            return Ok(Some(
                attribute(event, "link", "state")?.as_deref() != Some("down"),
            ));
        }
    }
    Ok(Some(true))
}

/// The XML of the interface on `tap` in domain XML `xml`, with its link state set to `up`.
/// `None` when there is no interface on `tap`.
///
/// The interface's events are written back as they were read, so every element, attribute,
/// value, comment and space keeps its bytes; only space inside an end tag is dropped. The one
/// change is the interface's own `link` element: one it has is replaced, and one it lacks is
/// added before its end tag. Nothing is read into a model of the interface, so nothing libvirt
/// added that Sherpa does not know of is lost: libvirt refuses a live update that changes a
/// field it cannot change live.
fn interface_with_link(xml: &str, tap: &str, up: bool) -> Result<Option<String>> {
    let Some(events) = find_interface(xml, tap)? else {
        return Ok(None);
    };
    let state = if up { "up" } else { "down" };
    let link = BytesStart::new("link").with_attributes([("state", state)]);
    let mut writer = Writer::new(Vec::new());
    let mut written = false;
    // Inside an old `<link>` with an end tag, whose events are dropped with it.
    let mut skipping = false;
    for (depth, event) in events {
        if skipping {
            skipping = !(depth == 1 && matches!(event, Event::End(_)));
            continue;
        }
        let old_link = depth == 1 && opens(&event, "link");
        let closes = depth == 0 && matches!(event, Event::End(_));
        if old_link || (closes && !written) {
            writer
                .write_event(Event::Empty(link.borrow()))
                .context("Cannot write the interface XML")?;
            written = true;
        }
        if old_link {
            skipping = matches!(event, Event::Start(_));
            continue;
        }
        writer
            .write_event(event)
            .context("Cannot write the interface XML")?;
    }
    String::from_utf8(writer.into_inner())
        .map(Some)
        .context("The interface XML is not UTF-8")
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

    /// A running domain's interface as libvirt writes it: indented, with an escaped value, a
    /// comment and a nested element.
    const LIVE: &str = "<domain type='kvm' id='7'>
  <name>r1-lab</name>
  <devices>
    <interface type='ethernet'>
      <mac address='52:54:00:00:00:02'/>
      <target dev='tpa0' managed='yes'/>
      <model type='virtio'/>
      <driver name='vhost' queues='4'>
        <host csum=\"off\" tso4='off'/>
      </driver>
      <!-- kept as it is -->
      <alias name='ua-a&amp;b&#39;c'/>
      <link state='down'>
      </link>
      <address type='pci' domain='0x0000' bus='0x01' slot='0x00' function='0x0'/>
    </interface>
  </devices>
</domain>
";

    #[test]
    fn an_interface_has_its_link_up_unless_its_link_says_down() {
        assert_eq!(link_up_in(DOMAIN, "tpa0").unwrap(), Some(true));
        assert_eq!(link_up_in(DOMAIN, "tpa1").unwrap(), Some(false));
        assert_eq!(link_up_in(LIVE, "tpa0").unwrap(), Some(false));
        assert_eq!(link_up_in(DOMAIN, "tpa9").unwrap(), None);
    }

    #[test]
    fn setting_the_link_inserts_one_before_the_interface_closes() {
        let down = interface_with_link(DOMAIN, "tpa0", false).unwrap().unwrap();
        assert_eq!(
            down,
            "<interface type='ethernet'><mac address='52:54:00:00:00:02'/>\
             <target dev='tpa0' managed='yes'/><model type='virtio'/>\
             <alias name='net1'/><address type='pci' domain='0x0000' bus='0x01' slot='0x00' function='0x0'/>\
             <link state=\"down\"/></interface>"
        );
        assert_eq!(link_up_in(&down, "tpa0").unwrap(), Some(false));
    }

    #[test]
    fn setting_the_link_replaces_a_link_the_interface_has() {
        let up = interface_with_link(DOMAIN, "tpa1", true).unwrap().unwrap();
        assert_eq!(
            up,
            "<interface type='ethernet'><mac address='52:54:00:00:00:03'/>\
             <target dev='tpa1' managed='yes'/><model type='virtio'/><link state=\"up\"/>\
             </interface>"
        );
        assert_eq!(link_up_in(&up, "tpa1").unwrap(), Some(true));
    }

    #[test]
    fn setting_the_link_passes_everything_else_through_byte_for_byte() {
        let up = interface_with_link(LIVE, "tpa0", true).unwrap().unwrap();
        assert_eq!(
            up,
            "<interface type='ethernet'>
      <mac address='52:54:00:00:00:02'/>
      <target dev='tpa0' managed='yes'/>
      <model type='virtio'/>
      <driver name='vhost' queues='4'>
        <host csum=\"off\" tso4='off'/>
      </driver>
      <!-- kept as it is -->
      <alias name='ua-a&amp;b&#39;c'/>
      <link state=\"up\"/>
      <address type='pci' domain='0x0000' bus='0x01' slot='0x00' function='0x0'/>
    </interface>"
        );
    }

    #[test]
    fn setting_the_link_leaves_other_interfaces_alone() {
        let down = interface_with_link(DOMAIN, "tpa0", false).unwrap().unwrap();
        assert!(!down.contains("vnet0"), "{down}");
        assert!(!down.contains("tpa1"), "{down}");
        assert_eq!(link_up_in(&down, "vnet0").unwrap(), None);
        // Only the interface's own `target` and `link` count, not ones nested deeper.
        let nested = "<domain><devices>\
            <interface type='ethernet'><backend><target dev='tpa0'/></backend></interface>\
            <interface type='ethernet'><target dev='tpa0'/><driver><link state='down'/></driver>\
            </interface></devices></domain>";
        assert_eq!(link_up_in(nested, "tpa0").unwrap(), Some(true));
        assert_eq!(
            interface_with_link(nested, "tpa0", false).unwrap().unwrap(),
            "<interface type='ethernet'><target dev='tpa0'/><driver><link state='down'/></driver>\
             <link state=\"down\"/></interface>"
        );
    }

    #[test]
    fn an_end_tag_with_whitespace_still_closes_the_interface() {
        let xml = "<domain><devices><interface type='ethernet'><target dev='tpa0'/>\
            </interface ></devices></domain>";
        assert_eq!(
            interface_with_link(xml, "tpa0", false).unwrap().unwrap(),
            "<interface type='ethernet'><target dev='tpa0'/><link state=\"down\"/></interface>"
        );
    }

    #[test]
    fn a_tap_no_interface_is_on_has_no_link_to_set() {
        assert_eq!(interface_with_link(DOMAIN, "tpa9", false).unwrap(), None);
        assert_eq!(link_up_in(DOMAIN, "tpa9").unwrap(), None);
    }

    #[test]
    fn malformed_domain_xml_is_refused() {
        for xml in [
            "<domain",
            "<domain><devices></domain>",
            "<domain><devices><interface type='ethernet'><target dev='tpa0'/></interface>",
            "<domain><interface type='ethernet'><target dev='tpa0'/></interface></domain></x>",
            "<domain><interface type='ethernet'><target dev='tpa0' dev='tpa1'/></interface></domain>",
        ] {
            let error = format!("{:#}", interface_with_link(xml, "tpa0", false).unwrap_err());
            assert!(
                error.contains("Cannot parse the domain XML"),
                "{xml}: {error}"
            );
            let error = format!("{:#}", link_up_in(xml, "tpa0").unwrap_err());
            assert!(
                error.contains("Cannot parse the domain XML"),
                "{xml}: {error}"
            );
        }
    }
}
