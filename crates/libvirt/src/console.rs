//! A domain's serial console: a UNIX socket that only sherpad reaches, and the move of a domain
//! defined before onto its console sockets (the serial console's here, VNC's in `crate::vnc`).
//!
//! Domains are defined with `<serial type='unix'>` and `<console type='unix'>`, bound by libvirt
//! at a path sherpad chooses. A domain defined before had a telnet listener on TCP instead,
//! reachable by any account on the server; [`move_consoles_to_sockets`] rewrites its stored
//! definition so it comes up on sockets at its next start. A running domain keeps the
//! character device it was started with until it stops, which [`serial_console`] reports.
//!
//! The rewrite reads the definition with quick-xml and writes every event back as it was read,
//! changing only the `serial` and `console` elements on TCP: their `type`, their `source`, and
//! their `protocol`, which goes. Nothing is read into a model of the domain, so nothing libvirt
//! added that Sherpa does not know of is lost.

use anyhow::{Context, Result, bail};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use tracing::instrument;
use virt::connect::Connect;
use virt::domain::Domain;
use virt::error::ErrorNumber;
use virt::sys::{VIR_DOMAIN_XML_INACTIVE, VIR_DOMAIN_XML_SECURE};

use crate::interface::{attribute, live_xml, opens};
use crate::vnc::graphics_on_socket;

/// The character devices of a domain that carry its serial console.
const CHARDEVS: [&str; 2] = ["serial", "console"];

/// The UNIX sockets of a domain's consoles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleSockets {
    /// The serial console's.
    pub serial: String,
    /// The VNC console's. A domain with no VNC graphics, such as a unikernel, has none bound.
    pub vnc: String,
}

/// How a domain's serial console is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerialConsole {
    /// A UNIX socket at this path, as Sherpa defines domains.
    Socket(String),
    /// A TCP listener, as domains defined before consoles moved behind sherpad have.
    Tcp,
    /// Another kind of character device, named by its type, such as `pty`.
    Other(String),
    /// No serial device.
    Absent,
}

/// How the serial console of running domain `domain` is reached, as it was started. `None` when
/// the domain does not exist or is not running.
///
/// # Errors
///
/// Fails when libvirt cannot look the domain up or read its live XML, or the XML cannot be
/// parsed.
#[instrument(level = "debug", skip(conn))]
pub fn serial_console(conn: &Connect, domain: &str) -> Result<Option<SerialConsole>> {
    let Some(xml) = live_xml(conn, domain)? else {
        return Ok(None);
    };
    serial_console_in(&xml).map(Some)
}

/// Rewrite the stored definition of domain `domain` so its serial and VNC consoles are the
/// UNIX sockets of `sockets` from its next start, those still on TCP. Returns whether the
/// definition changed. A running domain keeps its TCP listeners until it stops.
///
/// # Errors
///
/// Fails when the domain does not exist, libvirt cannot read its definition or refuses the new
/// one, or the definition cannot be parsed.
#[instrument(level = "debug", skip(conn))]
pub fn move_consoles_to_sockets(
    conn: &Connect,
    domain: &str,
    sockets: &ConsoleSockets,
) -> Result<bool> {
    let handle = match Domain::lookup_by_name(conn, domain) {
        Ok(handle) => handle,
        Err(error) if error.code() == ErrorNumber::NoDomain => {
            bail!("Domain '{domain}' does not exist")
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot find domain '{domain}'"));
        }
    };
    // The secure flag keeps any secret the definition holds, so defining it again loses none.
    let xml = handle
        .get_xml_desc(VIR_DOMAIN_XML_INACTIVE | VIR_DOMAIN_XML_SECURE)
        .with_context(|| format!("Cannot read the definition of domain '{domain}'"))?;
    let Some(rewritten) = consoles_on_sockets(&xml, sockets)? else {
        return Ok(false);
    };
    Domain::define_xml(conn, &rewritten)
        .with_context(|| format!("Cannot define domain '{domain}' with its consoles on sockets"))?;
    tracing::info!(%domain, ?sockets, "Moved the domain's consoles to UNIX sockets");
    Ok(true)
}

/// Domain XML `xml` with its serial and VNC consoles on TCP moved to `sockets`, `None` when
/// neither is on TCP.
fn consoles_on_sockets(xml: &str, sockets: &ConsoleSockets) -> Result<Option<String>> {
    let serial = serial_on_socket(xml, &sockets.serial)?;
    let vnc = graphics_on_socket(serial.as_deref().unwrap_or(xml), &sockets.vnc)?;
    Ok(vnc.or(serial))
}

/// How the serial console of domain XML `xml` is reached: its first `serial` element.
fn serial_console_in(xml: &str) -> Result<SerialConsole> {
    let mut reader = Reader::from_str(xml);
    // Depth of the serial element being read, once found.
    let mut serial = None;
    let mut kind = String::new();
    let mut open = 0usize;
    loop {
        let event = read(&mut reader)?;
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
            Event::Eof => break,
            _ => open,
        };
        match serial {
            None if opens(&event, "serial") => {
                kind = attribute(&event, "serial", "type")?
                    .unwrap_or_default()
                    .into_owned();
                if matches!(event, Event::Empty(_)) {
                    break;
                }
                serial = Some(depth);
            }
            Some(start) if depth == start + 1 && kind == "unix" => {
                if let Some(path) = attribute(&event, "source", "path")? {
                    return Ok(SerialConsole::Socket(path.into_owned()));
                }
            }
            Some(start) if depth == start && matches!(event, Event::End(_)) => break,
            _ => {}
        }
    }
    Ok(match kind.as_str() {
        "" if serial.is_none() => SerialConsole::Absent,
        "tcp" => SerialConsole::Tcp,
        _ => SerialConsole::Other(kind),
    })
}

/// Domain XML `xml` with each `serial` and `console` element on TCP moved to the UNIX socket at
/// `path`: its `type` becomes `unix`, its `source` keeps only its `mode` and gains `path`, and
/// its `protocol` goes. `None` when there is none on TCP: already on a socket, or no serial
/// device.
///
/// Every other event is written back as it was read, so every element, attribute, value,
/// comment and space keeps its bytes; only space inside an end tag is dropped, and the space
/// before a removed `protocol` goes with it. The attributes of a rewritten element are written
/// with double quotes.
fn serial_on_socket(xml: &str, path: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    let mut writer = Writer::new(Vec::new());
    let mut changed = false;
    let mut open = 0usize;
    // Depth of the TCP chardev being rewritten.
    let mut chardev = None;
    // Depth of an element whose events are dropped, until its end tag.
    let mut dropping = None;
    // Space read inside a chardev, written once the next event is known to be kept.
    let mut space = None;
    loop {
        let event = read(&mut reader)?;
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
        if let Some(start) = dropping {
            if depth == start && matches!(event, Event::End(_)) {
                dropping = None;
            }
            continue;
        }
        let Some(start) = chardev else {
            if let Some(name) = tcp_chardev(&event)? {
                let (Event::Start(element) | Event::Empty(element)) = &event else {
                    continue;
                };
                let unix = with_type_unix(element, name)?;
                let rewritten = if matches!(event, Event::Start(_)) {
                    chardev = Some(depth);
                    Event::Start(unix)
                } else {
                    Event::Empty(unix)
                };
                write(&mut writer, rewritten)?;
                changed = true;
            } else {
                write(&mut writer, event)?;
            }
            continue;
        };
        if let Event::Text(text) = &event
            && text.as_ref().trim_ascii().is_empty()
        {
            if let Some(held) = space.replace(event) {
                write(&mut writer, held)?;
            }
            continue;
        }
        let child = depth == start + 1;
        if child && opens(&event, "protocol") {
            space = None;
            if matches!(event, Event::Start(_)) {
                dropping = Some(depth);
            }
            continue;
        }
        if let Some(held) = space.take() {
            write(&mut writer, held)?;
        }
        if child && opens(&event, "source") {
            let (Event::Start(element) | Event::Empty(element)) = &event else {
                continue;
            };
            write(&mut writer, Event::Empty(socket_source(element, path)?))?;
            if matches!(event, Event::Start(_)) {
                dropping = Some(depth);
            }
            continue;
        }
        if depth == start && matches!(event, Event::End(_)) {
            chardev = None;
        }
        write(&mut writer, event)?;
    }
    if !changed {
        return Ok(None);
    }
    String::from_utf8(writer.into_inner())
        .map(Some)
        .context("The domain XML is not UTF-8")
}

/// The next event of `reader`, with every attribute of an element checked, so XML that is not
/// well formed is refused wherever it fails.
pub(crate) fn read<'a>(reader: &mut Reader<&'a [u8]>) -> Result<Event<'a>> {
    let event = reader.read_event().with_context(|| {
        format!(
            "Cannot parse the domain XML at byte {}",
            reader.error_position()
        )
    })?;
    if let Event::Start(element) | Event::Empty(element) = &event {
        for attribute in element.attributes() {
            attribute.context("Cannot parse the domain XML")?;
        }
    }
    Ok(event)
}

/// Write `event` to `writer`.
pub(crate) fn write(writer: &mut Writer<Vec<u8>>, event: Event) -> Result<()> {
    writer
        .write_event(event)
        .context("Cannot write the domain XML")
}

/// The name of the character device `event` opens when it is a serial console on TCP.
fn tcp_chardev(event: &Event) -> Result<Option<&'static str>> {
    for name in CHARDEVS {
        if attribute(event, name, "type")?.as_deref() == Some("tcp") {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Chardev `element`, named `name`, with its type `unix` and its other attributes as they were.
fn with_type_unix(element: &BytesStart, name: &str) -> Result<BytesStart<'static>> {
    let mut unix = BytesStart::new(name.to_owned());
    for attribute in element.attributes() {
        let attribute = attribute.context("Cannot parse the domain XML")?;
        if attribute.key.as_ref() == "type" {
            unix.push_attribute(("type", "unix"));
        } else {
            unix.push_attribute(attribute);
        }
    }
    Ok(unix)
}

/// A `source` element for the socket at `path`, keeping the `mode` of `element`: `bind`, as
/// QEMU listens and sherpad connects.
fn socket_source(element: &BytesStart, path: &str) -> Result<BytesStart<'static>> {
    let mut source = BytesStart::new("source");
    for attribute in element.attributes() {
        let attribute = attribute.context("Cannot parse the domain XML")?;
        if attribute.key.as_ref() == "mode" {
            source.push_attribute(attribute);
        }
    }
    source.push_attribute(("path", path));
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vnc::tests::{PATH as VNC_PATH, TCP as VNC_TCP, UNIX as VNC_UNIX};
    use crate::vnc::{VncConsole, vnc_console_in};

    /// A domain defined before consoles moved behind sherpad, as libvirt writes it back.
    const TCP: &str = "<domain type='kvm'>
  <name>alp01-abcd1234</name>
  <devices>
    <emulator>/usr/bin/qemu-system-x86_64</emulator>
    <!-- a comment, kept -->
    <serial type='tcp'>
      <source mode='bind' host='127.127.1.1' service='2323' tls='no'/>
      <protocol type='telnet'/>
      <target type='isa-serial' port='0'>
        <model name='isa-serial'/>
      </target>
      <alias name='serial0'/>
    </serial>
    <console type='tcp'>
      <source mode='bind' host='127.127.1.1' service='2323'/>
      <protocol type='telnet'/>
      <target type='serial' port='0'/>
      <alias name='serial0'/>
    </console>
    <graphics type='vnc' port='5901' autoport='no' listen='127.0.0.1'>
      <listen type='address' address='127.0.0.1'/>
    </graphics>
    <memballoon model='virtio'><alias name='a&amp;b&#39;c'/></memballoon>
  </devices>
</domain>
";

    /// [`TCP`] with its console on the socket: only the chardevs differ, and the attributes of the
    /// elements rewritten are written with double quotes.
    const UNIX: &str = r#"<domain type='kvm'>
  <name>alp01-abcd1234</name>
  <devices>
    <emulator>/usr/bin/qemu-system-x86_64</emulator>
    <!-- a comment, kept -->
    <serial type="unix">
      <source mode="bind" path="/opt/sherpa/run/console/abcd1234/1.serial"/>
      <target type='isa-serial' port='0'>
        <model name='isa-serial'/>
      </target>
      <alias name='serial0'/>
    </serial>
    <console type="unix">
      <source mode="bind" path="/opt/sherpa/run/console/abcd1234/1.serial"/>
      <target type='serial' port='0'/>
      <alias name='serial0'/>
    </console>
    <graphics type='vnc' port='5901' autoport='no' listen='127.0.0.1'>
      <listen type='address' address='127.0.0.1'/>
    </graphics>
    <memballoon model='virtio'><alias name='a&amp;b&#39;c'/></memballoon>
  </devices>
</domain>
"#;

    const PATH: &str = "/opt/sherpa/run/console/abcd1234/1.serial";

    #[test]
    fn console_upgrade_001_a_tcp_serial_console_moves_to_the_socket_and_nothing_else_changes() {
        assert_eq!(serial_on_socket(TCP, PATH).unwrap().as_deref(), Some(UNIX));
    }

    #[test]
    fn console_upgrade_004_a_definition_on_sockets_or_with_no_console_is_left_as_it_is() {
        assert_eq!(serial_on_socket(UNIX, PATH).unwrap(), None);
        let xml =
            "<domain type='kvm'><name>n</name><devices><disk type='file'/></devices></domain>";
        assert_eq!(serial_on_socket(xml, PATH).unwrap(), None);
        assert_eq!(serial_console_in(xml).unwrap(), SerialConsole::Absent);
        assert_eq!(graphics_on_socket(VNC_UNIX, VNC_PATH).unwrap(), None);
        // As libvirt writes a running domain's: the path in an attribute as well.
        let live = "<domain><devices><graphics type='vnc' socket='/s'>\
                    <listen type='socket' socket='/s'/></graphics></devices></domain>";
        assert_eq!(graphics_on_socket(live, VNC_PATH).unwrap(), None);
        assert_eq!(
            vnc_console_in(live).unwrap(),
            VncConsole::Socket("/s".to_owned())
        );
        for xml in [
            "<domain type='kvm'><name>n</name><devices><disk type='file'/></devices></domain>",
            "<domain><devices><graphics type='spice' port='5930'/></devices></domain>",
        ] {
            assert_eq!(graphics_on_socket(xml, VNC_PATH).unwrap(), None, "{xml}");
            assert_eq!(vnc_console_in(xml).unwrap(), VncConsole::Absent, "{xml}");
        }
    }

    #[test]
    fn console_upgrade_005_the_elements_are_rewritten_in_each_form_libvirt_and_sherpa_wrote_them() {
        let xml = "<domain><devices><serial type='tcp'><source mode='bind' host='h' \
                   service='1'><reconnect enabled='no'/></source><protocol type='telnet'>\
                   </protocol><target port='0'/></serial></devices></domain>";
        assert_eq!(
            serial_on_socket(xml, "/s").unwrap().as_deref(),
            Some(
                r#"<domain><devices><serial type="unix"><source mode="bind" path="/s"/><target port='0'/></serial></devices></domain>"#
            )
        );
        let xml = "<domain><devices>\n    <graphics type='vnc' port='5905' autoport='no'/>\n    \
                   <video/></devices></domain>";
        assert_eq!(
            graphics_on_socket(xml, "/s").unwrap().as_deref(),
            Some(
                r#"<domain><devices>
    <graphics type="vnc"><listen type="socket" socket="/s"/></graphics>
    <video/></devices></domain>"#
            )
        );
        let xml = "<domain><devices><graphics type='vnc' port='1' websocket='2'>\
                   <listen type='network' network='n'></listen><image compression='off'/>\
                   </graphics></devices></domain>";
        assert_eq!(
            graphics_on_socket(xml, "/s").unwrap().as_deref(),
            Some(
                r#"<domain><devices><graphics type="vnc"><listen type="socket" socket="/s"/><image compression='off'/></graphics></devices></domain>"#
            )
        );
    }

    #[test]
    fn console_upgrade_007_a_running_nodes_console_is_read_from_its_first_serial_and_vnc_device() {
        assert_eq!(serial_console_in(TCP).unwrap(), SerialConsole::Tcp);
        assert_eq!(
            serial_console_in(UNIX).unwrap(),
            SerialConsole::Socket(PATH.to_owned())
        );
        let pty =
            "<domain><devices><serial type='pty'><target port='0'/></serial></devices></domain>";
        assert_eq!(
            serial_console_in(pty).unwrap(),
            SerialConsole::Other("pty".to_owned())
        );
        assert_eq!(vnc_console_in(VNC_TCP).unwrap(), VncConsole::Tcp);
        assert_eq!(
            vnc_console_in(VNC_UNIX).unwrap(),
            VncConsole::Socket(VNC_PATH.to_owned())
        );
    }

    #[test]
    fn console_upgrade_003_both_consoles_move_in_one_definition_and_one_moves_alone() {
        let sockets = ConsoleSockets {
            serial: PATH.to_owned(),
            vnc: "/opt/sherpa/run/console/abcd1234/1.vnc".to_owned(),
        };
        let both = consoles_on_sockets(TCP, &sockets).unwrap().unwrap();
        let expected = UNIX.replace(
            "<graphics type='vnc' port='5901' autoport='no' listen='127.0.0.1'>
      <listen type='address' address='127.0.0.1'/>",
            r#"<graphics type="vnc">
      <listen type="socket" socket="/opt/sherpa/run/console/abcd1234/1.vnc"/>"#,
        );
        assert_eq!(both, expected);
        assert_eq!(consoles_on_sockets(&both, &sockets).unwrap(), None);
        // Only the VNC console left on TCP: it moves alone.
        assert_eq!(
            consoles_on_sockets(UNIX, &sockets).unwrap().as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn console_upgrade_006_a_definition_that_is_not_well_formed_xml_is_refused() {
        assert!(serial_on_socket("<domain><devices><serial type='tcp'>", PATH).is_err());
        assert!(serial_on_socket("<domain a='1' a='2'></domain>", PATH).is_err());
        assert!(serial_console_in("<domain></devices>").is_err());
        assert!(graphics_on_socket("<domain><devices><graphics type='vnc'>", VNC_PATH).is_err());
        assert!(graphics_on_socket("<domain a='1' a='2'></domain>", VNC_PATH).is_err());
        assert!(vnc_console_in("<domain><graphics type='vnc'></domain>").is_err());
    }
}
