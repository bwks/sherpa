/// Integration tests for the libvirt crate.
///
/// These tests require a running libvirt daemon with QEMU/KVM support.
/// Run: cargo test -p libvirt -- --ignored --test-threads=1
///
/// Tests create and destroy their own networks and storage pools.
/// Resource names are prefixed with "sherpa-test-" to avoid collisions.
use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;

use anyhow::Result;
use virt::connect::Connect;
use virt::domain::Domain;
use virt::network::Network;
use virt::storage_pool::StoragePool;

use libvirt::{
    BridgeNetwork, IsolatedNetwork, NatNetwork, Qemu, QemuConnection, ReservedNetwork,
    SherpaStoragePool, host_cpus, host_memory, list_domain_memory,
};

// ============================================================================
// Helper
// ============================================================================

fn connect() -> Result<QemuConnection> {
    Qemu::default().connect()
}

/// Clean up a libvirt network by name. Ignores errors (network may not exist).
fn cleanup_network(conn: &Connect, name: &str) {
    if let Ok(net) = Network::lookup_by_name(conn, name) {
        let _ = net.destroy();
        let _ = net.undefine();
    }
}

/// Stop and remove a domain by name. Ignores errors (the domain may not exist).
fn cleanup_domain(conn: &Connect, name: &str) {
    if let Ok(domain) = Domain::lookup_by_name(conn, name) {
        let _ = domain.destroy();
        let _ = domain.undefine();
    }
}

/// Clean up a libvirt storage pool by name. Ignores errors.
fn cleanup_pool(conn: &Connect, name: &str) {
    if let Ok(pool) = StoragePool::lookup_by_name(conn, name) {
        let _ = pool.destroy();
        let _ = pool.undefine();
    }
}

// ============================================================================
// QEMU Connection
// ============================================================================

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_qemu_connect() {
    let conn = connect().expect("connects to QEMU");
    let hostname = conn.get_hostname().expect("gets hostname");
    assert_ne!(hostname, "");
}

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_qemu_connect_default_uri() {
    let qemu = Qemu::default();
    let conn = qemu.connect().expect("connects");
    // Verify we can query something
    let version = conn.get_lib_version().expect("gets version");
    assert!(version > 0);
}

// ============================================================================
// Network — Bridge
// ============================================================================

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_create_bridge_network() {
    let conn = connect().expect("connects");
    let name = "sherpa-test-br-net";
    let bridge = "sherpa-tbr0";

    cleanup_network(&conn, name);

    BridgeNetwork {
        network_name: name.to_string(),
        bridge_name: bridge.to_string(),
    }
    .create(&conn)
    .expect("creates bridge network");

    // Verify network exists
    let net = Network::lookup_by_name(&conn, name).expect("network should exist");
    assert!(net.is_active().expect("checks active"));

    // Cleanup
    let _ = net.destroy();
    let _ = net.undefine();
}

// ============================================================================
// Network — Isolated
// ============================================================================

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_create_isolated_network() {
    let conn = connect().expect("connects");
    let name = "sherpa-test-iso-net";
    let bridge = "sherpa-tiso0";

    cleanup_network(&conn, name);

    IsolatedNetwork {
        network_name: name.to_string(),
        bridge_name: bridge.to_string(),
    }
    .create(&conn)
    .expect("creates isolated network");

    let net = Network::lookup_by_name(&conn, name).expect("network should exist");
    assert!(net.is_active().expect("checks active"));

    let xml = net.get_xml_desc(0).expect("gets xml");
    // libvirt omits <forward> element when mode is 'none'
    assert!(
        !xml.contains("<forward"),
        "should not have forward element (no forwarding)"
    );
    assert!(xml.contains("isolated='yes'"), "ports should be isolated");

    let _ = net.destroy();
    let _ = net.undefine();
}

// ============================================================================
// Network — Reserved
// ============================================================================

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_create_reserved_network() {
    let conn = connect().expect("connects");
    let name = "sherpa-test-rsv-net";
    let bridge = "sherpa-trsv0";

    cleanup_network(&conn, name);

    ReservedNetwork {
        network_name: name.to_string(),
        bridge_name: bridge.to_string(),
    }
    .create(&conn)
    .expect("creates reserved network");

    let net = Network::lookup_by_name(&conn, name).expect("network should exist");
    assert!(net.is_active().expect("checks active"));

    let xml = net.get_xml_desc(0).expect("gets xml");
    assert!(
        !xml.contains("<forward"),
        "should not have forward element (no forwarding)"
    );
    assert!(
        xml.contains("isolated='no'"),
        "ports should NOT be isolated"
    );

    let _ = net.destroy();
    let _ = net.undefine();
}

// ============================================================================
// Network — NAT
// ============================================================================

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_create_nat_network_ipv4() {
    let conn = connect().expect("connects");
    let name = "sherpa-test-nat-net";
    let bridge = "sherpa-tnat0";

    cleanup_network(&conn, name);

    NatNetwork {
        network_name: name.to_string(),
        bridge_name: bridge.to_string(),
        ipv4_address: Ipv4Addr::new(192, 168, 250, 1),
        ipv4_netmask: Ipv4Addr::new(255, 255, 255, 0),
        ipv6_address: None,
        ipv6_prefix_length: None,
    }
    .create(&conn)
    .expect("creates NAT network");

    let net = Network::lookup_by_name(&conn, name).expect("network should exist");
    assert!(net.is_active().expect("checks active"));

    let xml = net.get_xml_desc(0).expect("gets xml");
    assert!(xml.contains("mode='nat'"), "should be NAT mode");
    assert!(xml.contains("192.168.250.1"), "should contain IPv4 address");
    assert!(
        !xml.contains("family='ipv6'"),
        "should not contain IPv6 when not provided"
    );

    let _ = net.destroy();
    let _ = net.undefine();
}

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_create_nat_network_dual_stack() {
    let conn = connect().expect("connects");
    let name = "sherpa-test-nat6-net";
    let bridge = "sherpa-tnat6";

    cleanup_network(&conn, name);

    NatNetwork {
        network_name: name.to_string(),
        bridge_name: bridge.to_string(),
        ipv4_address: Ipv4Addr::new(192, 168, 251, 1),
        ipv4_netmask: Ipv4Addr::new(255, 255, 255, 0),
        ipv6_address: Some("fd00:abcd::1".parse().expect("valid")),
        ipv6_prefix_length: Some(64),
    }
    .create(&conn)
    .expect("creates dual-stack NAT network");

    let net = Network::lookup_by_name(&conn, name).expect("network should exist");
    let xml = net.get_xml_desc(0).expect("gets xml");
    assert!(xml.contains("192.168.251.1"));
    assert!(xml.contains("family='ipv6'"), "should contain IPv6 block");

    let _ = net.destroy();
    let _ = net.undefine();
}

// ============================================================================
// Network — Idempotency
// ============================================================================

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_network_create_idempotent() {
    let conn = connect().expect("connects");
    let name = "sherpa-test-idem-net";
    let bridge = "sherpa-tidem";

    cleanup_network(&conn, name);

    let net1 = IsolatedNetwork {
        network_name: name.to_string(),
        bridge_name: bridge.to_string(),
    };
    let net2 = IsolatedNetwork {
        network_name: name.to_string(),
        bridge_name: bridge.to_string(),
    };

    // Create twice — second call should not error
    net1.create(&conn).expect("first create");
    net2.create(&conn).expect("second create (idempotent)");

    // Still active
    let net = Network::lookup_by_name(&conn, name).expect("network should exist");
    assert!(net.is_active().expect("checks active"));

    let _ = net.destroy();
    let _ = net.undefine();
}

// ============================================================================
// Storage Pool
// ============================================================================

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_create_storage_pool() {
    let conn = connect().expect("connects");
    let pool_name = "sherpa-test-pool";
    let pool_path = "/tmp/sherpa-test-pool";

    cleanup_pool(&conn, pool_name);
    let _ = fs::remove_dir_all(pool_path);

    SherpaStoragePool {
        name: pool_name.to_string(),
        path: pool_path.to_string(),
    }
    .create(&conn)
    .expect("creates storage pool");

    // Verify pool exists and is active
    let pool = StoragePool::lookup_by_name(&conn, pool_name).expect("pool should exist");
    assert!(pool.is_active().expect("checks active"));

    // Verify directory was created
    assert!(Path::new(pool_path).exists(), "pool directory should exist");

    // Cleanup
    let _ = pool.destroy();
    let _ = pool.undefine();
    let _ = fs::remove_dir_all(pool_path);
}

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_create_storage_pool_idempotent() {
    let conn = connect().expect("connects");
    let pool_name = "sherpa-test-pool-idem";
    let pool_path = "/tmp/sherpa-test-pool-idem";

    cleanup_pool(&conn, pool_name);
    let _ = fs::remove_dir_all(pool_path);

    let pool1 = SherpaStoragePool {
        name: pool_name.to_string(),
        path: pool_path.to_string(),
    };
    let pool2 = SherpaStoragePool {
        name: pool_name.to_string(),
        path: pool_path.to_string(),
    };

    pool1.create(&conn).expect("first create");
    pool2.create(&conn).expect("second create (idempotent)");

    let pool = StoragePool::lookup_by_name(&conn, pool_name).expect("pool should exist");
    assert!(pool.is_active().expect("checks active"));

    let _ = pool.destroy();
    let _ = pool.undefine();
    let _ = fs::remove_dir_all(pool_path);
}

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_create_storage_pool_starts_an_inactive_pool() {
    let conn = connect().expect("connects");
    let pool_name = "sherpa-test-pool-inactive";
    let pool_path = "/tmp/sherpa-test-pool-inactive";

    cleanup_pool(&conn, pool_name);
    let _ = fs::remove_dir_all(pool_path);

    SherpaStoragePool {
        name: pool_name.to_string(),
        path: pool_path.to_string(),
    }
    .create(&conn)
    .expect("first create");
    let defined = StoragePool::lookup_by_name(&conn, pool_name).expect("pool should exist");
    defined.destroy().expect("stops the pool");

    SherpaStoragePool {
        name: pool_name.to_string(),
        path: pool_path.to_string(),
    }
    .create(&conn)
    .expect("second create");
    assert!(defined.is_active().expect("checks active"));

    let _ = defined.destroy();
    let _ = defined.undefine();
    let _ = fs::remove_dir_all(pool_path);
}

// ============================================================================
// Host and domain memory
// ============================================================================

#[test]
#[ignore = "requires a libvirt daemon"]
fn test_host_memory_matches_the_numa_cells() {
    let conn = connect().expect("connects");
    let memory = host_memory(&conn).expect("reads the host memory");
    assert!(memory.total_kib > 0);
    assert_ne!(memory.cells.len(), 0);
    let cells_total: u64 = memory.cells.iter().map(|cell| cell.total_kib).sum();
    assert!(cells_total > 0);
    // Each cell's memory is its share of the host's; together they are within 10% of it.
    assert!(cells_total.abs_diff(memory.total_kib) * 10 <= memory.total_kib);
    assert!(memory.hugepages_kib <= memory.total_kib);
    let cpus = host_cpus(&conn).expect("reads the host CPUs");
    assert_eq!(cpus.numa_nodes as usize, memory.cells.len());
}

/// Defines, starts and removes a 64 MiB domain held strictly on NUMA node 0, with no disks.
#[test]
#[ignore = "requires a libvirt daemon with KVM"]
fn test_domain_memory_reports_size_state_and_strict_numa_node() {
    let conn = connect().expect("connects");
    let name = "sherpa-test-memory";
    cleanup_domain(&conn, name);
    let xml = format!(
        "<domain type='kvm'><name>{name}</name><memory unit='MiB'>64</memory>\
         <vcpu>1</vcpu><numatune><memory mode='strict' nodeset='0'/></numatune>\
         <os><type arch='x86_64'>hvm</type></os><devices/></domain>"
    );
    let domain = Domain::define_xml(&conn, &xml).expect("defines the domain");

    let find = |conn: &Connect| {
        list_domain_memory(conn)
            .expect("lists domain memory")
            .into_iter()
            .find(|entry| entry.name == name)
            .expect("lists the test domain")
    };
    let defined = find(&conn);
    assert!(!defined.active);
    assert_eq!(defined.memory_kib, 64 * 1024);
    assert_eq!(defined.strict_numa_node, Some(0));

    let started = domain.create();
    let running = started.as_ref().ok().map(|_| find(&conn));
    cleanup_domain(&conn, name);
    started.expect("starts the domain");
    let running = running.expect("reads the running domain");
    assert!(running.active);
    assert_eq!(running.memory_kib, 64 * 1024);
    assert_eq!(running.strict_numa_node, Some(0));
}

/// Defines, starts and removes a 64 MiB domain that prefers NUMA node 0, with no disks. Its
/// memory may go to another node, so it is not bound to one, defined or running.
#[test]
#[ignore = "requires a libvirt daemon with KVM"]
fn test_domain_memory_preferring_a_numa_node_is_not_bound_to_it() {
    let conn = connect().expect("connects");
    let name = "sherpa-test-memory-preferred";
    cleanup_domain(&conn, name);
    let xml = format!(
        "<domain type='kvm'><name>{name}</name><memory unit='MiB'>64</memory>\
         <vcpu>1</vcpu><numatune><memory mode='preferred' nodeset='0'/></numatune>\
         <os><type arch='x86_64'>hvm</type></os><devices/></domain>"
    );
    let domain = Domain::define_xml(&conn, &xml).expect("defines the domain");

    let find = |conn: &Connect| {
        list_domain_memory(conn)
            .expect("lists domain memory")
            .into_iter()
            .find(|entry| entry.name == name)
            .expect("lists the test domain")
    };
    let defined = find(&conn);
    assert!(!defined.active);
    assert_eq!(defined.strict_numa_node, None);

    let started = domain.create();
    let running = started.as_ref().ok().map(|_| find(&conn));
    cleanup_domain(&conn, name);
    started.expect("starts the domain");
    let running = running.expect("reads the running domain");
    assert!(running.active);
    assert_eq!(running.strict_numa_node, None);
}
