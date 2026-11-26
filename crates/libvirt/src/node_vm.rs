//! Node VM domains: the kernel command line of a defined domain, and waiting for one to stop.

use std::io;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use tracing::instrument;
use virt::connect::Connect;
use virt::domain::Domain;
use virt::sys::{VIR_DOMAIN_XML_INACTIVE, VIR_DOMAIN_XML_SECURE};

use crate::error::LibvirtError;

/// The pause between two checks of whether a domain has stopped.
const STOPPED_POLL: Duration = Duration::from_secs(1);

/// Domain XML `domain_xml` with the text of its `<os><cmdline>` replaced by `cmdline`; every
/// other part is kept as it is.
///
/// # Errors
///
/// Fails when the XML cannot be parsed or written, or has no `<cmdline>` in `<os>`.
fn with_cmdline(domain_xml: &str, cmdline: &str) -> Result<String> {
    let mut reader = Reader::from_str(domain_xml);
    let mut writer = Writer::new(Vec::new());
    let mut path: Vec<String> = Vec::new();
    let mut replaced = false;
    loop {
        let position = reader.buffer_position();
        let event = reader
            .read_event()
            .with_context(|| LibvirtError::DomainXmlInvalidAt { position })?;
        let in_os = path.len() == 2 && path[1] == "os";
        let in_cmdline = path.len() == 3 && path[1] == "os" && path[2] == "cmdline";
        let written = match event {
            Event::Eof => break,
            Event::Start(element) if in_os && element.name().as_ref() == "cmdline" => {
                path.push("cmdline".to_owned());
                replaced = true;
                write_cmdline(&mut writer, cmdline, false)
            }
            Event::Empty(element) if in_os && element.name().as_ref() == "cmdline" => {
                replaced = true;
                write_cmdline(&mut writer, cmdline, true)
            }
            Event::Text(_) | Event::CData(_) if in_cmdline => Ok(()),
            Event::Start(element) => {
                path.push(element.name().as_ref().to_owned());
                writer.write_event(Event::Start(element))
            }
            Event::End(element) => {
                path.pop();
                writer.write_event(Event::End(element))
            }
            other => writer.write_event(other),
        };
        written.context(LibvirtError::DomainXmlWrite)?;
    }
    if !replaced {
        return Err(LibvirtError::DomainXmlNoCmdline.into());
    }
    String::from_utf8(writer.into_inner()).context(LibvirtError::DomainXmlWrite)
}

/// Write `<cmdline>` with text `cmdline`, and its end tag when `close`.
fn write_cmdline(writer: &mut Writer<Vec<u8>>, cmdline: &str, close: bool) -> io::Result<()> {
    writer.write_event(Event::Start(BytesStart::new("cmdline")))?;
    writer.write_event(Event::Text(BytesText::new(cmdline)))?;
    if close {
        writer.write_event(Event::End(BytesEnd::new("cmdline")))?;
    }
    Ok(())
}

/// Set the kernel command line of defined domain `name` to `cmdline` from its next boot on; a
/// running domain keeps the one it booted with.
///
/// # Errors
///
/// Fails when libvirt cannot find the domain, read its XML or define it again, or the XML has
/// no kernel command line.
#[instrument(level = "debug", skip(conn))]
pub fn set_node_vm_cmdline(conn: &Connect, name: &str, cmdline: &str) -> Result<()> {
    let domain = Domain::lookup_by_name(conn, name)
        .with_context(|| format!("Cannot look up domain {name}"))?;
    let xml = domain
        .get_xml_desc(VIR_DOMAIN_XML_INACTIVE | VIR_DOMAIN_XML_SECURE)
        .with_context(|| format!("Cannot read the XML of domain {name}"))?;
    Domain::define_xml(conn, &with_cmdline(&xml, cmdline)?)
        .with_context(|| format!("Cannot define domain {name} again"))?;
    Ok(())
}

/// Wait until domain `name` has stopped, for up to `timeout`.
///
/// # Errors
///
/// Fails when libvirt cannot find the domain or read whether it runs, or it still runs after
/// `timeout`.
#[instrument(level = "debug", skip(conn))]
pub fn wait_until_stopped(conn: &Connect, name: &str, timeout: Duration) -> Result<()> {
    let domain = Domain::lookup_by_name(conn, name)
        .with_context(|| format!("Cannot look up domain {name}"))?;
    let started = Instant::now();
    while domain
        .is_active()
        .with_context(|| format!("Cannot read whether domain {name} runs"))?
    {
        if started.elapsed() >= timeout {
            return Err(LibvirtError::StopTimedOut {
                name: name.to_owned(),
                seconds: timeout.as_secs(),
            }
            .into());
        }
        thread::sleep(STOPPED_POLL);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN: &str = "<domain type='kvm'><name>ceos1</name><os><type>hvm</type>\
        <kernel>/k</kernel><cmdline>root=LABEL=ROOT flatcar.first_boot=detected</cmdline></os>\
        <devices><disk/></devices></domain>";

    #[test]
    fn node_vm_032_the_kernel_command_line_is_replaced_and_the_rest_is_kept() {
        let replaced = with_cmdline(DOMAIN, "root=LABEL=ROOT a&b").unwrap();
        assert_eq!(
            replaced,
            DOMAIN.replace(
                "root=LABEL=ROOT flatcar.first_boot=detected",
                "root=LABEL=ROOT a&amp;b"
            )
        );
        let empty = "<domain><os><type>hvm</type><cmdline/></os></domain>";
        assert_eq!(
            with_cmdline(empty, "x").unwrap(),
            "<domain><os><type>hvm</type><cmdline>x</cmdline></os></domain>"
        );
        let refused = with_cmdline(
            "<domain><os/><devices><cmdline>x</cmdline></devices></domain>",
            "y",
        )
        .unwrap_err();
        assert_eq!(
            refused.downcast_ref::<LibvirtError>(),
            Some(&LibvirtError::DomainXmlNoCmdline)
        );
    }
}
