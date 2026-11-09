use anyhow::{Context, Result, bail};
use roxmltree::{Document, Node};
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
    let document =
        Document::parse(capabilities).context("The server's capabilities are not valid XML")?;
    let Some(cells) = document
        .descendants()
        .find(|node| node.has_tag_name("cells"))
    else {
        return Ok(Vec::new());
    };
    cells
        .children()
        .filter(|node| node.has_tag_name("cell"))
        .map(parse_numa_cell)
        .collect()
}

/// One `<cell>` of the capabilities: its id, memory and hugepage pools.
fn parse_numa_cell(cell: Node) -> Result<NumaCell> {
    let id: u32 = cell
        .attribute("id")
        .context("A NUMA cell in the capabilities has no id")?
        .trim()
        .parse()
        .context("A NUMA cell id in the capabilities is not a number")?;
    let total_kib = match child(cell, "memory") {
        Some(memory) => to_kib(number(memory.text(), "memory")?, memory.attribute("unit"))
            .with_context(|| format!("Cannot read NUMA node {id}'s memory"))?,
        None => 0,
    };
    let mut pages = Vec::new();
    for pool in cell.children().filter(|node| node.has_tag_name("pages")) {
        let size = number(pool.attribute("size"), "page size")
            .and_then(|size| to_kib(size, pool.attribute("unit")))
            .with_context(|| format!("Cannot read NUMA node {id}'s page size"))?;
        let count = number(pool.text(), "page count")
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
    Ok(NumaCell {
        id,
        total_kib,
        hugepages_kib,
    })
}

/// The first child element of `node` named `tag`.
fn child<'a, 'input>(node: Node<'a, 'input>, tag: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|child| child.has_tag_name(tag))
}

/// A whole number from an element's text or an attribute; `what` names it in the error.
fn number(value: Option<&str>, what: &str) -> Result<u64> {
    let value = value.map(str::trim).unwrap_or_default();
    if value.is_empty() {
        bail!("The {what} has no value");
    }
    value
        .parse()
        .with_context(|| format!("The {what} '{value}' is not a whole number"))
}

/// `value` in `unit`, converted to KiB and rounded down. The units are libvirt's: `b`/`bytes`,
/// `KB`/`MB`/... in powers of 1000, and `K`/`KiB`/`M`/`MiB`/... in powers of 1024, with the
/// letter in either case. A value without a unit is in KiB, as libvirt writes it.
fn to_kib(value: u64, unit: Option<&str>) -> Result<u64> {
    let Some(unit) = unit else {
        return Ok(value);
    };
    let bytes_per_unit =
        unit_bytes(unit).with_context(|| format!("Unknown memory unit '{unit}'"))?;
    let kib = u128::from(value) * bytes_per_unit / 1024;
    u64::try_from(kib).with_context(|| format!("{value} {unit} is too large"))
}

/// The bytes in one `unit`, or `None` when libvirt does not know the unit.
fn unit_bytes(unit: &str) -> Option<u128> {
    let lower = unit.to_ascii_lowercase();
    if matches!(lower.as_str(), "b" | "byte" | "bytes") {
        return Some(1);
    }
    let mut letters = lower.chars();
    let power = match letters.next()? {
        'k' => 1,
        'm' => 2,
        'g' => 3,
        't' => 4,
        'p' => 5,
        'e' => 6,
        _ => return None,
    };
    let base: u128 = match letters.as_str() {
        "" | "ib" => 1024,
        "b" => 1000,
        _ => return None,
    };
    Some(base.pow(power))
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
    fn memory_in_other_units_is_converted_to_kib() {
        let xml = "<cells num='1'><cell id='0'>\
            <memory unit='MiB'>2</memory>\
            <pages unit='b' size='4096'>100</pages>\
            <pages unit='GiB' size='1'>3</pages>\
            </cell></cells>";
        assert_eq!(
            parse_numa_cells(xml).unwrap(),
            vec![NumaCell {
                id: 0,
                total_kib: 2048,
                hugepages_kib: 3 * 1_048_576,
            }]
        );
    }

    #[test]
    fn libvirt_units_scale_by_1000_or_1024() {
        assert_eq!(to_kib(5, None).unwrap(), 5);
        assert_eq!(to_kib(5, Some("KiB")).unwrap(), 5);
        assert_eq!(to_kib(5, Some("k")).unwrap(), 5);
        assert_eq!(to_kib(2048, Some("bytes")).unwrap(), 2);
        assert_eq!(to_kib(1024, Some("KB")).unwrap(), 1000);
        assert_eq!(to_kib(1, Some("G")).unwrap(), 1_048_576);
        assert_eq!(to_kib(1, Some("gb")).unwrap(), 976_562);
        assert_eq!(to_kib(1, Some("TiB")).unwrap(), 1_073_741_824);
    }

    #[test]
    fn an_unknown_memory_unit_is_refused() {
        let xml = "<cells num='1'><cell id='0'><memory unit='pages'>1</memory></cell></cells>";
        let error = format!("{:#}", parse_numa_cells(xml).unwrap_err());
        assert!(error.contains("NUMA node 0's memory"), "{error}");
        assert!(error.contains("Unknown memory unit 'pages'"), "{error}");
    }

    #[test]
    fn a_cell_without_an_id_is_refused() {
        let xml = "<cells num='1'><cell><memory unit='KiB'>1</memory></cell></cells>";
        let error = format!("{:#}", parse_numa_cells(xml).unwrap_err());
        assert!(error.contains("has no id"), "{error}");
    }

    #[test]
    fn a_page_pool_without_a_size_is_refused() {
        let xml = "<cells num='1'><cell id='1'><pages unit='KiB'>1</pages></cell></cells>";
        let error = format!("{:#}", parse_numa_cells(xml).unwrap_err());
        assert!(error.contains("NUMA node 1's page size"), "{error}");
        assert!(error.contains("page size has no value"), "{error}");
    }

    #[test]
    fn invalid_xml_is_refused() {
        let xml = "<capabilities><host><topology><cells num='1'><cell id='0'></cells>";
        let error = format!("{:#}", parse_numa_cells(xml).unwrap_err());
        assert!(error.contains("not valid XML"), "{error}");
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
}
