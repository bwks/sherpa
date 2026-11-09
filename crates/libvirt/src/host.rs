use anyhow::{Context, Result, bail};
use tracing::instrument;
use virt::connect::Connect;
use virt::domain::Domain;
use virt::error::ErrorNumber;
use virt::sys::{VIR_DOMAIN_NUMATUNE_MEM_RESTRICTIVE, VIR_DOMAIN_NUMATUNE_MEM_STRICT};

/// The server's online CPUs and NUMA nodes, as libvirt reports them. Both are numbered from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostCpus {
    pub cpus: u32,
    pub numa_nodes: u32,
}

/// One host NUMA node's memory, from the capabilities XML.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumaCell {
    pub id: u32,
    /// All the node's memory, in KiB, including its hugepage pool.
    pub total_kib: u64,
    /// Memory the node holds in reserved hugepages, in KiB. Guests that do not ask for
    /// hugepages cannot use it.
    pub hugepages_kib: u64,
}

/// The server's memory: its total, its hugepage pools and each NUMA node's share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostMemory {
    /// All the server's memory, in KiB, including the hugepage pools.
    pub total_kib: u64,
    /// Memory held in reserved hugepages on every node, in KiB.
    pub hugepages_kib: u64,
    /// The NUMA nodes, in the order libvirt reports them. Never empty: a server that reports
    /// no NUMA topology is one node holding all its memory.
    pub cells: Vec<NumaCell>,
}

/// The memory a domain is configured with, and where libvirt must hold it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainMemory {
    pub name: String,
    /// Whether the domain is running or paused, so its memory is in use.
    pub active: bool,
    /// The domain's maximum memory, in KiB.
    pub memory_kib: u64,
    /// The one host NUMA node the domain's memory must stay on, when its `numatune` mode is
    /// `strict` or `restrictive` and its nodeset is a single node. Memory that may fall back
    /// to other nodes, or spread over several, is not bound to one.
    pub strict_numa_node: Option<u32>,
}

/// The server's CPUs and NUMA nodes. The NUMA node count comes from the capabilities XML;
/// `virNodeGetInfo` may report 1 for topologies it does not describe.
///
/// # Errors
///
/// Fails when libvirt cannot report the host's CPUs or capabilities.
#[instrument(level = "debug", skip(qemu_conn))]
pub fn host_cpus(qemu_conn: &Connect) -> Result<HostCpus> {
    let info = qemu_conn
        .get_node_info()
        .context("Failed to read the server's CPUs from libvirt")?;
    let capabilities = qemu_conn
        .get_capabilities()
        .context("Failed to read the server's capabilities from libvirt")?;
    let cells = parse_numa_cells(&capabilities)?;
    Ok(HostCpus {
        cpus: info.cpus,
        numa_nodes: u32::try_from(cells.len()).unwrap_or(u32::MAX).max(1),
    })
}

/// The server's memory, from `virNodeGetInfo` and the capabilities XML.
///
/// # Errors
///
/// Fails when libvirt cannot report the host's memory or capabilities, or the capabilities
/// XML cannot be read.
#[instrument(level = "debug", skip(qemu_conn))]
pub fn host_memory(qemu_conn: &Connect) -> Result<HostMemory> {
    let info = qemu_conn
        .get_node_info()
        .context("Failed to read the server's memory from libvirt")?;
    let capabilities = qemu_conn
        .get_capabilities()
        .context("Failed to read the server's capabilities from libvirt")?;
    Ok(host_memory_from(
        info.memory,
        parse_numa_cells(&capabilities)?,
    ))
}

/// The memory of every domain libvirt knows, running or not, Sherpa's and anyone else's. A
/// domain removed while it is read is left out.
///
/// # Errors
///
/// Fails when libvirt cannot list the domains or read one's memory.
#[instrument(level = "debug", skip(qemu_conn))]
pub fn list_domain_memory(qemu_conn: &Connect) -> Result<Vec<DomainMemory>> {
    let domains = qemu_conn
        .list_all_domains(0)
        .context("Failed to list the server's domains")?;
    let mut memory = Vec::with_capacity(domains.len());
    for domain in domains {
        if let Some(entry) = domain_memory(&domain)? {
            memory.push(entry);
        }
    }
    Ok(memory)
}

/// One domain's memory, or `None` when it no longer exists.
fn domain_memory(domain: &Domain) -> Result<Option<DomainMemory>> {
    let name = match domain.get_name() {
        Ok(name) => name,
        Err(error) if error.code() == ErrorNumber::NoDomain => return Ok(None),
        Err(error) => return Err(error).context("Failed to read a domain's name"),
    };
    let info = match domain.get_info() {
        Ok(info) => info,
        Err(error) if error.code() == ErrorNumber::NoDomain => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read domain '{name}' memory"));
        }
    };
    let active = match domain.is_active() {
        Ok(active) => active,
        Err(error) if error.code() == ErrorNumber::NoDomain => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to read domain '{name}' state"));
        }
    };
    // Without NUMA parameters, the memory still counts against the whole server.
    let strict_numa_node = match domain.get_numa_parameters(0) {
        Ok(params) => strict_numa_node(params.mode, params.node_set.as_deref()),
        Err(error) => {
            tracing::warn!(domain = %name, %error, "Cannot read the domain's NUMA placement");
            None
        }
    };
    Ok(Some(DomainMemory {
        name,
        active,
        memory_kib: info.max_mem,
        strict_numa_node,
    }))
}

/// The host's memory from its total and NUMA cells. A host without cells is one node.
fn host_memory_from(total_kib: u64, cells: Vec<NumaCell>) -> HostMemory {
    let cells = if cells.is_empty() {
        vec![NumaCell {
            id: 0,
            total_kib,
            hugepages_kib: 0,
        }]
    } else {
        cells
    };
    HostMemory {
        total_kib,
        hugepages_kib: cells.iter().map(|cell| cell.hugepages_kib).sum(),
        cells,
    }
}

/// The node a domain's memory is bound to: its nodeset when that is one node and the mode
/// gives the memory nowhere else to go.
fn strict_numa_node(mode: Option<i32>, nodeset: Option<&str>) -> Option<u32> {
    let mode = u32::try_from(mode?).ok()?;
    if mode != VIR_DOMAIN_NUMATUNE_MEM_STRICT && mode != VIR_DOMAIN_NUMATUNE_MEM_RESTRICTIVE {
        return None;
    }
    nodeset?.trim().parse().ok()
}

/// The NUMA cells in libvirt's capabilities XML (`<host><topology><cells>`), with each cell's
/// memory and hugepage pools. Empty when the XML describes no cells.
fn parse_numa_cells(capabilities: &str) -> Result<Vec<NumaCell>> {
    let Some((_, cells)) = elements(capabilities, "cells").into_iter().next() else {
        return Ok(Vec::new());
    };
    let mut parsed = Vec::new();
    for (attributes, body) in elements(cells, "cell") {
        let id: u32 = attribute(attributes, "id")
            .context("A NUMA cell in the capabilities has no id")?
            .parse()
            .context("A NUMA cell id in the capabilities is not a number")?;
        let total_kib = match elements(body, "memory").into_iter().next() {
            Some((memory_attributes, value)) => kib(memory_attributes, value)
                .with_context(|| format!("Cannot read NUMA node {id}'s memory"))?,
            None => 0,
        };
        let mut pages = Vec::new();
        for (page_attributes, count) in elements(body, "pages") {
            let size = kib(
                page_attributes,
                attribute(page_attributes, "size").unwrap_or(""),
            )
            .with_context(|| format!("Cannot read NUMA node {id}'s page size"))?;
            let count: u64 = count
                .trim()
                .parse()
                .with_context(|| format!("Cannot read NUMA node {id}'s page count"))?;
            pages.push((size, count));
        }
        // The smallest page is the base page; the larger ones are hugepage pools.
        let base = pages.iter().map(|(size, _)| *size).min().unwrap_or(0);
        let hugepages_kib = pages
            .iter()
            .filter(|(size, _)| *size > base)
            .map(|(size, count)| size.saturating_mul(*count))
            .fold(0u64, u64::saturating_add);
        parsed.push(NumaCell {
            id,
            total_kib,
            hugepages_kib,
        });
    }
    Ok(parsed)
}

/// A value in KiB, from an element whose `unit` attribute must be KiB, as libvirt writes it.
fn kib(attributes: &str, value: &str) -> Result<u64> {
    if let Some(unit) = attribute(attributes, "unit")
        && unit != "KiB"
    {
        bail!("Unexpected memory unit '{unit}'");
    }
    value
        .trim()
        .parse()
        .with_context(|| format!("'{}' is not a number of KiB", value.trim()))
}

/// The attributes and body of each `<tag>` element in `xml`, in order. Elements of the same
/// tag must not nest, which holds for the capabilities elements read here. A self-closing
/// element has an empty body.
fn elements<'a>(xml: &'a str, tag: &str) -> Vec<(&'a str, &'a str)> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after_name = &rest[start + open.len()..];
        // `<cells` must not match `<cell`, so the name ends at a space, `>` or `/`.
        if !after_name.starts_with([' ', '>', '/', '\t', '\n', '\r']) {
            rest = after_name;
            continue;
        }
        let Some(tag_end) = after_name.find('>') else {
            break;
        };
        let attributes = &after_name[..tag_end];
        let after_tag = &after_name[tag_end + 1..];
        if attributes.ends_with('/') {
            found.push((attributes.trim_end_matches('/'), ""));
            rest = after_tag;
            continue;
        }
        let Some(body_end) = after_tag.find(&close) else {
            break;
        };
        found.push((attributes, &after_tag[..body_end]));
        rest = &after_tag[body_end + close.len()..];
    }
    found
}

/// An attribute's value, in single or double quotes.
fn attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    for quote in ['\'', '"'] {
        let key = format!(" {name}={quote}");
        if let Some(start) = attributes.find(&key) {
            let value = &attributes[start + key.len()..];
            return value.find(quote).map(|end| &value[..end]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_NODES: &str = "<capabilities>
  <host>
    <cpu>
      <topology sockets='1' dies='1' cores='8' threads='2'/>
    </cpu>
    <topology>
      <cells num='2'>
        <cell id='0'>
          <memory unit='KiB'>83886080</memory>
          <pages unit='KiB' size='4'>20447232</pages>
          <pages unit='KiB' size='2048'>1024</pages>
          <pages unit='KiB' size='1048576'>0</pages>
          <distances>
            <sibling id='0' value='10'/>
            <sibling id='1' value='21'/>
          </distances>
          <cpus num='2'>
            <cpu id='0' socket_id='0' core_id='0' siblings='0,16'/>
            <cpu id='16' socket_id='0' core_id='0' siblings='0,16'/>
          </cpus>
        </cell>
        <cell id='1'>
          <memory unit='KiB'>82837504</memory>
          <pages unit='KiB' size='4'>20709376</pages>
          <pages unit='KiB' size='2048'>0</pages>
          <pages unit='KiB' size='1048576'>2</pages>
          <cpus num='1'>
            <cpu id='8' socket_id='1' core_id='0' siblings='8'/>
          </cpus>
        </cell>
      </cells>
    </topology>
  </host>
  <guest>
    <arch name='x86_64'>
      <domain type='kvm'/>
    </arch>
  </guest>
</capabilities>";

    #[test]
    fn reads_each_numa_cells_memory_and_hugepage_pools() {
        let cells = parse_numa_cells(TWO_NODES).unwrap();
        assert_eq!(
            cells,
            vec![
                NumaCell {
                    id: 0,
                    total_kib: 83_886_080,
                    hugepages_kib: 1024 * 2048,
                },
                NumaCell {
                    id: 1,
                    total_kib: 82_837_504,
                    hugepages_kib: 2 * 1_048_576,
                },
            ]
        );
    }

    #[test]
    fn capabilities_without_cells_describe_one_node_with_all_the_memory() {
        let cells = parse_numa_cells("<capabilities><host><cpu/></host></capabilities>").unwrap();
        assert_eq!(cells, []);
        let memory = host_memory_from(4096, cells);
        assert_eq!(memory.cells.len(), 1);
        assert_eq!(memory.cells[0].total_kib, 4096);
        assert_eq!(memory.hugepages_kib, 0);
    }

    #[test]
    fn host_hugepages_are_the_sum_of_every_cells() {
        let memory = host_memory_from(166_723_584, parse_numa_cells(TWO_NODES).unwrap());
        assert_eq!(memory.hugepages_kib, 1024 * 2048 + 2 * 1_048_576);
        assert_eq!(memory.total_kib, 166_723_584);
    }

    #[test]
    fn a_memory_unit_other_than_kib_is_refused() {
        let xml = "<cells num='1'><cell id='0'><memory unit='MiB'>1</memory></cell></cells>";
        let error = parse_numa_cells(xml).unwrap_err();
        assert!(format!("{error:#}").contains("Unexpected memory unit 'MiB'"));
    }

    #[test]
    fn a_memoryless_cell_has_none() {
        let xml = "<cells num='1'><cell id='3'><cpus num='0'></cpus></cell></cells>";
        assert_eq!(
            parse_numa_cells(xml).unwrap(),
            vec![NumaCell {
                id: 3,
                total_kib: 0,
                hugepages_kib: 0,
            }]
        );
    }

    #[test]
    fn only_strict_or_restrictive_single_node_memory_is_bound() {
        let strict = i32::try_from(VIR_DOMAIN_NUMATUNE_MEM_STRICT).unwrap();
        let restrictive = i32::try_from(VIR_DOMAIN_NUMATUNE_MEM_RESTRICTIVE).unwrap();
        let preferred = 1;
        let interleave = 2;
        assert_eq!(strict_numa_node(Some(strict), Some("1")), Some(1));
        assert_eq!(strict_numa_node(Some(restrictive), Some(" 0 ")), Some(0));
        assert_eq!(strict_numa_node(Some(preferred), Some("1")), None);
        assert_eq!(strict_numa_node(Some(interleave), Some("1")), None);
        assert_eq!(strict_numa_node(Some(strict), Some("0-1")), None);
        assert_eq!(strict_numa_node(Some(strict), Some("0,1")), None);
        assert_eq!(strict_numa_node(Some(strict), Some("")), None);
        assert_eq!(strict_numa_node(Some(strict), None), None);
        assert_eq!(strict_numa_node(None, Some("1")), None);
    }

    #[test]
    fn elements_do_not_match_a_longer_tag_name() {
        let xml = "<cells num='1'><cell id='0'/></cells>";
        assert_eq!(elements(xml, "cell"), vec![(" id='0'", "")]);
        assert_eq!(elements(xml, "cells").len(), 1);
    }

    #[test]
    fn attributes_take_either_quote() {
        assert_eq!(attribute(" id=\"2\" unit='KiB'", "id"), Some("2"));
        assert_eq!(attribute(" id=\"2\" unit='KiB'", "unit"), Some("KiB"));
        assert_eq!(attribute(" sid='2'", "id"), None);
    }
}
