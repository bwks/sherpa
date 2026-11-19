//! A domain's VNC console: a UNIX socket that only sherpad reaches.
//!
//! Domains are defined with `<graphics type='vnc'>` listening on a socket at a path sherpad
//! chooses (`<listen type='socket'>`). A domain defined before had a TCP port on the server's
//! loopback instead, reachable by any account on the server; [`graphics_on_socket`] moves its
//! stored definition to the socket, as `crate::move_consoles_to_sockets` does it with the serial
//! console. A running domain keeps the listener it was started with until it stops, which
//! [`vnc_console`] reports.
//!
//! As for the serial console, the rewrite writes every event back as it was read, changing only
//! the `graphics` element of type `vnc` that is not on a socket.

use std::borrow::Cow;

use anyhow::{Context, Result, bail};
use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::events::{BytesStart, Event};
use tracing::instrument;
use virt::connect::Connect;

use crate::console::{read, write};
use crate::interface::{attribute, live_xml, opens};

/// The attributes of a VNC `graphics` element that place it on TCP, dropped when it moves to a
/// socket: its port, its WebSocket port and its listen address.
const TCP_ATTRIBUTES: [&str; 5] = ["port", "autoport", "websocket", "listen", "socket"];

/// How a domain's VNC console is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VncConsole {
    /// A UNIX socket at this path, as Sherpa defines domains.
    Socket(String),
    /// A TCP port, as domains defined before VNC moved behind sherpad have.
    Tcp,
    /// No VNC graphics.
    Absent,
}

/// How the VNC console of running domain `domain` is reached, as it was started. `None` when the
/// domain does not exist or is not running.
///
/// # Errors
///
/// Fails when libvirt cannot look the domain up or read its live XML, or the XML cannot be
/// parsed.
#[instrument(level = "debug", skip(conn))]
pub fn vnc_console(conn: &Connect, domain: &str) -> Result<Option<VncConsole>> {
    let Some(xml) = live_xml(conn, domain)? else {
        return Ok(None);
    };
    vnc_console_in(&xml).map(Some)
}

/// How the VNC console of domain XML `xml` is reached: its first VNC `graphics` element.
pub(crate) fn vnc_console_in(xml: &str) -> Result<VncConsole> {
    let mut reader = Reader::from_str(xml);
    loop {
        let event = read(&mut reader)?;
        if matches!(event, Event::Eof) {
            return Ok(VncConsole::Absent);
        }
        if is_vnc(&event)? {
            let graphics = element(&mut reader, event)?;
            return Ok(match socket_of(&graphics)? {
                Some(path) => VncConsole::Socket(path),
                None => VncConsole::Tcp,
            });
        }
    }
}

/// Domain XML `xml` with each VNC `graphics` element not on a socket moved to the UNIX socket
/// at `path`: its port, WebSocket and listen attributes go, as do its `listen` children, and a
/// `<listen type='socket'>` child takes their place. `None` when there is none to move: already
/// on a socket, or no VNC graphics.
///
/// Every other event is written back as it was read. In the element rewritten, the space before
/// a removed `listen` goes with it, and attributes are written with double quotes.
pub(crate) fn graphics_on_socket(xml: &str, path: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(xml);
    let mut writer = Writer::new(Vec::new());
    let mut changed = false;
    let mut open = 0usize;
    loop {
        let event = read(&mut reader)?;
        match &event {
            Event::Eof if open > 0 => {
                bail!("Cannot parse the domain XML: it ends inside an element")
            }
            Event::Eof => break,
            Event::Start(_) if !is_vnc(&event)? => open += 1,
            Event::End(_) => {
                open = open
                    .checked_sub(1)
                    .context("Cannot parse the domain XML: an end tag closes nothing")?;
            }
            _ => {}
        }
        if !is_vnc(&event)? {
            write(&mut writer, event)?;
            continue;
        }
        let graphics = element(&mut reader, event)?;
        if socket_of(&graphics)?.is_some() {
            for event in graphics {
                write(&mut writer, event)?;
            }
        } else {
            write_on_socket(&mut writer, &graphics, path)?;
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    String::from_utf8(writer.into_inner())
        .map(Some)
        .context("The domain XML is not UTF-8")
}

/// Whether `event` opens a `graphics` element of type `vnc`.
fn is_vnc(event: &Event) -> Result<bool> {
    Ok(attribute(event, "graphics", "type")?.as_deref() == Some("vnc"))
}

/// The events of the element `first` opens, `first` and its end tag included, read from
/// `reader`: `first` alone when it is empty.
fn element<'a>(reader: &mut Reader<&'a [u8]>, first: Event<'a>) -> Result<Vec<Event<'a>>> {
    let empty = matches!(first, Event::Empty(_));
    let mut events = vec![first];
    if empty {
        return Ok(events);
    }
    let mut open = 1usize;
    while open > 0 {
        let event = read(reader)?;
        match event {
            Event::Start(_) => open += 1,
            Event::End(_) => open -= 1,
            Event::Eof => bail!("Cannot parse the domain XML: it ends inside an element"),
            _ => {}
        }
        events.push(event);
    }
    Ok(events)
}

/// The socket path of the VNC `graphics` element whose events are `graphics`: its `socket`
/// attribute, or the `socket` of a `<listen type='socket'>` child. `None` when it is on TCP.
fn socket_of(graphics: &[Event]) -> Result<Option<String>> {
    let Some(first) = graphics.first() else {
        return Ok(None);
    };
    if let Some(path) = attribute(first, "graphics", "socket")? {
        return Ok(Some(path.into_owned()));
    }
    for (depth, event) in depths(graphics) {
        if depth == 1 && attribute(event, "listen", "type")?.as_deref() == Some("socket") {
            return Ok(attribute(event, "listen", "socket")?.map(Cow::into_owned));
        }
    }
    Ok(None)
}

/// Write the VNC `graphics` element whose events are `graphics` to `writer`, moved to the socket
/// at `path`.
fn write_on_socket(writer: &mut Writer<Vec<u8>>, graphics: &[Event], path: &str) -> Result<()> {
    let mut events = depths(graphics).collect::<Vec<_>>().into_iter();
    let Some((_, first)) = events.next() else {
        return Ok(());
    };
    let (Event::Start(element) | Event::Empty(element)) = first else {
        return Ok(());
    };
    write(writer, Event::Start(on_socket(element)?))?;
    // The new listen takes the indent of the first child, so it sits where the old one did.
    let indent = graphics.get(1).and_then(|event| match event {
        Event::Text(text) if text.as_ref().trim_ascii().is_empty() => Some(event.clone()),
        _ => None,
    });
    if let Some(indent) = indent {
        write(writer, indent)?;
    }
    let mut listen = BytesStart::new("listen");
    listen.push_attribute(("type", "socket"));
    listen.push_attribute(("socket", path));
    write(writer, Event::Empty(listen))?;
    if matches!(first, Event::Empty(_)) {
        return write(writer, Event::End(element.to_end().into_owned()));
    }
    // Space read, written once the next event is known to be kept.
    let mut space = None;
    // Depth of a `listen` child whose events are dropped, until its end tag.
    let mut dropping = false;
    for (depth, event) in events {
        if dropping {
            dropping = !(depth == 1 && matches!(event, Event::End(_)));
            continue;
        }
        if let Event::Text(text) = event
            && text.as_ref().trim_ascii().is_empty()
        {
            if let Some(held) = space.replace(event.clone()) {
                write(writer, held)?;
            }
            continue;
        }
        if depth == 1 && opens(event, "listen") {
            space = None;
            dropping = matches!(event, Event::Start(_));
            continue;
        }
        if let Some(held) = space.take() {
            write(writer, held)?;
        }
        write(writer, event.clone())?;
    }
    Ok(())
}

/// Graphics `element` with the attributes that place it on TCP removed.
fn on_socket(element: &BytesStart) -> Result<BytesStart<'static>> {
    let mut graphics = BytesStart::new("graphics");
    for attribute in element.attributes() {
        let attribute = attribute.context("Cannot parse the domain XML")?;
        let key = attribute.key.as_ref();
        if !TCP_ATTRIBUTES.contains(&key) {
            graphics.push_attribute(attribute);
        }
    }
    Ok(graphics.into_owned())
}

/// Each of the events of one element, `events`, with its depth: 0 for the element's own start
/// and end tags, 1 for its children's, and so on.
fn depths<'e, 'a>(events: &'e [Event<'a>]) -> impl Iterator<Item = (usize, &'e Event<'a>)> {
    let mut open = 0usize;
    events.iter().map(move |event| {
        let depth = match event {
            Event::Start(_) => {
                open += 1;
                open - 1
            }
            Event::End(_) => {
                open = open.saturating_sub(1);
                open
            }
            _ => open,
        };
        (depth, event)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "/opt/sherpa/run/console/abcd1234/1.vnc";

    /// A domain defined before VNC moved behind sherpad, as libvirt writes it back.
    const TCP: &str = "<domain type='kvm'>
  <name>alp01-abcd1234</name>
  <devices>
    <!-- a comment, kept -->
    <serial type='unix'>
      <source mode='bind' path='/opt/sherpa/run/console/abcd1234/1.serial'/>
    </serial>
    <graphics type='vnc' port='5901' autoport='no' listen='127.0.0.1' keymap='en-us'>
      <listen type='address' address='127.0.0.1'/>
    </graphics>
    <video>
      <model type='cirrus' vram='16384' heads='1' primary='yes'/>
    </video>
    <memballoon model='virtio'><alias name='a&amp;b&#39;c'/></memballoon>
  </devices>
</domain>
";

    /// [`TCP`] with its VNC console on the socket: only the graphics element differs.
    const UNIX: &str = r#"<domain type='kvm'>
  <name>alp01-abcd1234</name>
  <devices>
    <!-- a comment, kept -->
    <serial type='unix'>
      <source mode='bind' path='/opt/sherpa/run/console/abcd1234/1.serial'/>
    </serial>
    <graphics type="vnc" keymap="en-us">
      <listen type="socket" socket="/opt/sherpa/run/console/abcd1234/1.vnc"/>
    </graphics>
    <video>
      <model type='cirrus' vram='16384' heads='1' primary='yes'/>
    </video>
    <memballoon model='virtio'><alias name='a&amp;b&#39;c'/></memballoon>
  </devices>
</domain>
"#;

    #[test]
    fn a_tcp_vnc_console_moves_to_the_socket_and_nothing_else_changes() {
        assert_eq!(
            graphics_on_socket(TCP, PATH).unwrap().as_deref(),
            Some(UNIX)
        );
    }

    #[test]
    fn a_vnc_console_already_on_a_socket_is_left_alone() {
        assert_eq!(graphics_on_socket(UNIX, PATH).unwrap(), None);
        // As libvirt writes a running domain's: the path in an attribute as well.
        let live = "<domain><devices><graphics type='vnc' socket='/s'>\
                    <listen type='socket' socket='/s'/></graphics></devices></domain>";
        assert_eq!(graphics_on_socket(live, PATH).unwrap(), None);
        assert_eq!(
            vnc_console_in(live).unwrap(),
            VncConsole::Socket("/s".to_owned())
        );
    }

    #[test]
    fn a_domain_with_no_vnc_graphics_is_left_alone() {
        for xml in [
            "<domain type='kvm'><name>n</name><devices><disk type='file'/></devices></domain>",
            "<domain><devices><graphics type='spice' port='5930'/></devices></domain>",
        ] {
            assert_eq!(graphics_on_socket(xml, PATH).unwrap(), None, "{xml}");
            assert_eq!(vnc_console_in(xml).unwrap(), VncConsole::Absent, "{xml}");
        }
    }

    #[test]
    fn an_empty_graphics_element_as_sherpa_wrote_it_gains_a_socket_listen() {
        let xml = "<domain><devices>\n    <graphics type='vnc' port='5905' autoport='no'/>\n    \
                   <video/></devices></domain>";
        assert_eq!(
            graphics_on_socket(xml, "/s").unwrap().as_deref(),
            Some(
                "<domain><devices>\n    <graphics type=\"vnc\"><listen type=\"socket\" \
                 socket=\"/s\"/></graphics>\n    <video/></devices></domain>"
            )
        );
    }

    #[test]
    fn listen_children_with_end_tags_and_other_children_are_handled() {
        let xml = "<domain><devices><graphics type='vnc' port='1' websocket='2'>\
                   <listen type='network' network='n'></listen><image compression='off'/>\
                   </graphics></devices></domain>";
        assert_eq!(
            graphics_on_socket(xml, "/s").unwrap().as_deref(),
            Some(
                "<domain><devices><graphics type=\"vnc\"><listen type=\"socket\" \
                 socket=\"/s\"/><image compression='off'/></graphics></devices></domain>"
            )
        );
    }

    #[test]
    fn a_path_is_escaped_as_an_attribute_value() {
        let xml = "<domain><devices><graphics type='vnc' port='1'/></devices></domain>";
        let rewritten = graphics_on_socket(xml, "/run/a&b").unwrap().unwrap();
        assert!(
            rewritten.contains(r#"socket="/run/a&amp;b""#),
            "{rewritten}"
        );
        assert_eq!(
            vnc_console_in(&rewritten).unwrap(),
            VncConsole::Socket("/run/a&b".to_owned())
        );
    }

    #[test]
    fn the_vnc_console_is_read_from_the_first_vnc_graphics() {
        assert_eq!(vnc_console_in(TCP).unwrap(), VncConsole::Tcp);
        assert_eq!(
            vnc_console_in(UNIX).unwrap(),
            VncConsole::Socket(PATH.to_owned())
        );
    }

    #[test]
    fn xml_that_is_not_well_formed_is_refused() {
        assert!(graphics_on_socket("<domain><devices><graphics type='vnc'>", PATH).is_err());
        assert!(graphics_on_socket("<domain a='1' a='2'></domain>", PATH).is_err());
        assert!(vnc_console_in("<domain><graphics type='vnc'></domain>").is_err());
    }
}
