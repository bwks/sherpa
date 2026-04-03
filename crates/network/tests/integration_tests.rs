/// Integration tests for the network crate.
///
/// These tests require root privileges or CAP_NET_ADMIN capability.
/// Run: sudo -E cargo test -p network -- --ignored --test-threads=1
///
/// eBPF tests additionally require CAP_BPF and CAP_PERFMON.
///
/// Tests create and destroy their own interfaces.
/// Interface names are prefixed with "st-" (sherpa-test) to avoid collisions.
/// Names are kept short — Linux interface names are limited to 15 characters.
use anyhow::Result;

use network::{
    attach_p2p_redirect, create_bridge, create_tap, create_veth_pair, delete_interface,
    enslave_to_bridge, find_interfaces_fuzzy, get_ifindex, apply_netem, remove_netem,
    LinkImpairment,
};

// ============================================================================
// Helper
// ============================================================================

/// Delete an interface, ignoring errors (it may not exist).
async fn cleanup_interface(name: &str) {
    let _ = delete_interface(name).await;
}

/// Returns the raw output of `ip link show <name>`, or empty string on failure.
fn ip_link_show(name: &str) -> String {
    std::process::Command::new("ip")
        .args(["link", "show", name])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Check whether the UP flag is set on an interface (via ip link show flags field).
fn interface_is_up(name: &str) -> bool {
    let out = ip_link_show(name);
    // Flags appear in angle brackets, e.g. <BROADCAST,MULTICAST,UP>
    out.contains(",UP") || out.contains("<UP")
}

/// Return the MTU of an interface, or None if not found.
fn interface_mtu(name: &str) -> Option<u32> {
    let out = ip_link_show(name);
    let parts: Vec<&str> = out.split_whitespace().collect();
    for window in parts.windows(2) {
        if window[0] == "mtu" {
            return window[1].parse().ok();
        }
    }
    None
}

// ============================================================================
// Bridge
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_create_bridge() -> Result<()> {
    let name = "st-br0";
    let alias = "sherpa-test-bridge";

    cleanup_interface(name).await;

    create_bridge(name, alias).await?;

    // Verify bridge exists via fuzzy search
    let found = find_interfaces_fuzzy(name).await?;
    assert!(
        found.iter().any(|n| n == name),
        "Bridge {} should exist, found: {:?}",
        name,
        found
    );

    // Cleanup
    delete_interface(name).await?;

    // Verify gone
    let found = find_interfaces_fuzzy(name).await?;
    assert!(
        !found.iter().any(|n| n == name),
        "Bridge should be gone after deletion"
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_bridge_mtu_and_up_state() -> Result<()> {
    let name = "st-br-mtu";
    let alias = "sherpa-test-mtu";

    cleanup_interface(name).await;

    create_bridge(name, alias).await?;

    assert_eq!(
        interface_mtu(name),
        Some(9600),
        "Bridge MTU should be 9600 (jumbo frames)"
    );

    assert!(
        interface_is_up(name),
        "Bridge should have UP flag set after creation"
    );

    delete_interface(name).await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_bridge_duplicate() -> Result<()> {
    let name = "st-br-dup";
    let alias = "sherpa-test-dup";

    cleanup_interface(name).await;

    create_bridge(name, alias).await?;

    let result = create_bridge(name, alias).await;
    assert!(
        result.is_err(),
        "Creating a duplicate bridge should fail with an error"
    );

    delete_interface(name).await?;

    Ok(())
}

// ============================================================================
// Veth pair
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_create_veth_pair() -> Result<()> {
    let src = "st-veth0a";
    let dst = "st-veth0b";

    cleanup_interface(src).await;
    cleanup_interface(dst).await;

    create_veth_pair(src, dst, "test-src-alias", "test-dst-alias").await?;

    // Verify both ends exist
    let found = find_interfaces_fuzzy("st-veth0").await?;
    assert!(found.iter().any(|n| n == src), "src veth should exist");
    assert!(found.iter().any(|n| n == dst), "dst veth should exist");

    // Cleanup — deleting one end removes both
    delete_interface(src).await?;

    let found = find_interfaces_fuzzy("st-veth0").await?;
    assert!(!found.iter().any(|n| n == src), "src should be gone");
    assert!(
        !found.iter().any(|n| n == dst),
        "dst should be gone (deleting one end removes both)"
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_veth_pair_up_state() -> Result<()> {
    let src = "st-veth-up-a";
    let dst = "st-veth-up-b";

    cleanup_interface(src).await;
    cleanup_interface(dst).await;

    create_veth_pair(src, dst, "test-up-src", "test-up-dst").await?;

    assert!(
        interface_is_up(src),
        "src veth should have UP flag set after creation"
    );
    assert!(
        interface_is_up(dst),
        "dst veth should have UP flag set after creation"
    );

    delete_interface(src).await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_veth_pair_duplicate() -> Result<()> {
    let src = "st-veth-dp-a";
    let dst = "st-veth-dp-b";

    cleanup_interface(src).await;
    cleanup_interface(dst).await;

    create_veth_pair(src, dst, "test-dup-src", "test-dup-dst").await?;

    let result = create_veth_pair(src, dst, "test-dup-src", "test-dup-dst").await;
    assert!(
        result.is_err(),
        "Creating a duplicate veth pair should fail with an error"
    );

    delete_interface(src).await?;

    Ok(())
}

// ============================================================================
// Enslave to bridge
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_enslave_veth_to_bridge() -> Result<()> {
    let br = "st-br1";
    let src = "st-veth1a";
    let dst = "st-veth1b";

    cleanup_interface(br).await;
    cleanup_interface(src).await;

    // Create bridge and veth pair
    create_bridge(br, "test-bridge-enslave").await?;
    create_veth_pair(src, dst, "test-enslave-src", "test-enslave-dst").await?;

    // Enslave src end to bridge
    enslave_to_bridge(src, br).await?;

    // If we got here without error, the enslave succeeded.
    // The interface is now a bridge port.

    // Cleanup
    delete_interface(src).await?;
    delete_interface(br).await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_enslave_nonexistent_interface_fails() -> Result<()> {
    let br = "st-br2";

    cleanup_interface(br).await;

    create_bridge(br, "test-bridge-noexist").await?;

    // Try to enslave an interface that doesn't exist
    let result = enslave_to_bridge("st-noexist", br).await;
    assert!(
        result.is_err(),
        "Enslaving nonexistent interface should fail"
    );

    delete_interface(br).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_enslave_to_nonexistent_bridge() -> Result<()> {
    let src = "st-veth3a";
    let dst = "st-veth3b";

    cleanup_interface(src).await;
    cleanup_interface(dst).await;

    create_veth_pair(src, dst, "test-enslv-src", "test-enslv-dst").await?;

    // Try to enslave to a bridge that doesn't exist
    let result = enslave_to_bridge(src, "st-nobridge").await;
    assert!(
        result.is_err(),
        "Enslaving to a nonexistent bridge should fail"
    );

    delete_interface(src).await?;

    Ok(())
}

// ============================================================================
// Delete interface
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_delete_nonexistent_interface_fails() -> Result<()> {
    let result = delete_interface("st-noexist99").await;
    assert!(
        result.is_err(),
        "Deleting nonexistent interface should fail"
    );
    Ok(())
}

// ============================================================================
// Fuzzy search
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_find_interfaces_fuzzy_no_matches() -> Result<()> {
    let found = find_interfaces_fuzzy("st-zzz-nonexistent").await?;
    assert!(found.is_empty(), "Should find no matches");
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_find_interfaces_fuzzy_multiple_matches() -> Result<()> {
    let br1 = "st-fz-br1";
    let br2 = "st-fz-br2";

    cleanup_interface(br1).await;
    cleanup_interface(br2).await;

    create_bridge(br1, "fuzzy-test-1").await?;
    create_bridge(br2, "fuzzy-test-2").await?;

    // Both should match "st-fz-"
    let found = find_interfaces_fuzzy("st-fz-").await?;
    assert!(
        found.len() >= 2,
        "Should find at least 2 matches, got: {:?}",
        found
    );
    assert!(found.iter().any(|n| n == br1));
    assert!(found.iter().any(|n| n == br2));

    delete_interface(br1).await?;
    delete_interface(br2).await?;

    Ok(())
}

// ============================================================================
// Tap device
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_create_tap() -> Result<()> {
    let name = "st-tap0";

    cleanup_interface(name).await;

    create_tap(name, "test-tap-alias").await?;

    let found = find_interfaces_fuzzy(name).await?;
    assert!(
        found.iter().any(|n| n == name),
        "Tap {} should exist, found: {:?}",
        name,
        found
    );

    assert!(
        interface_is_up(name),
        "Tap should have UP flag set after creation"
    );

    assert_eq!(
        interface_mtu(name),
        Some(9600),
        "Tap MTU should be 9600 (jumbo frames)"
    );

    delete_interface(name).await?;

    let found = find_interfaces_fuzzy(name).await?;
    assert!(
        !found.iter().any(|n| n == name),
        "Tap should be gone after deletion"
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_ifindex() -> Result<()> {
    let name = "st-tap-idx";

    cleanup_interface(name).await;

    create_tap(name, "test-ifindex").await?;

    let idx = get_ifindex(name).await?;
    assert!(idx > 0, "ifindex should be positive, got: {}", idx);

    delete_interface(name).await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_get_ifindex_nonexistent_fails() -> Result<()> {
    let result = get_ifindex("st-noexist99").await;
    assert!(result.is_err(), "Getting ifindex of nonexistent interface should fail");
    Ok(())
}

// ============================================================================
// TC netem impairment
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_apply_netem_to_veth() -> Result<()> {
    let src = "st-netem-a";
    let dst = "st-netem-b";

    cleanup_interface(src).await;

    create_veth_pair(src, dst, "netem-src", "netem-dst").await?;

    let idx = get_ifindex(src).await?;

    let impairment = LinkImpairment {
        delay_us: 50000, // 50ms
        jitter_us: 5000, // 5ms
        loss_percent: 1.0,
        reorder_percent: 0.0,
        corrupt_percent: 0.0,
    };

    apply_netem(idx as i32, &impairment).await?;

    // Verify netem qdisc was created via tc show
    let output = std::process::Command::new("tc")
        .args(["qdisc", "show", "dev", src])
        .output()
        .expect("tc command failed");
    let tc_output = String::from_utf8_lossy(&output.stdout);
    assert!(
        tc_output.contains("netem"),
        "tc qdisc show should contain 'netem', got: {}",
        tc_output
    );

    delete_interface(src).await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_remove_netem() -> Result<()> {
    let src = "st-netem-rm-a";
    let dst = "st-netem-rm-b";

    cleanup_interface(src).await;

    create_veth_pair(src, dst, "netem-rm-src", "netem-rm-dst").await?;

    let idx = get_ifindex(src).await?;

    let impairment = LinkImpairment {
        delay_us: 10000,
        jitter_us: 0,
        loss_percent: 0.0,
        reorder_percent: 0.0,
        corrupt_percent: 0.0,
    };

    apply_netem(idx as i32, &impairment).await?;
    remove_netem(idx as i32).await?;

    // Verify netem qdisc is gone
    let output = std::process::Command::new("tc")
        .args(["qdisc", "show", "dev", src])
        .output()
        .expect("tc command failed");
    let tc_output = String::from_utf8_lossy(&output.stdout);
    assert!(
        !tc_output.contains("netem"),
        "netem should be removed, got: {}",
        tc_output
    );

    delete_interface(src).await?;

    Ok(())
}

// ============================================================================
// eBPF P2p redirect
// Requires: CAP_NET_ADMIN + CAP_BPF + CAP_PERFMON
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_ebpf_redirect_between_veths() -> Result<()> {
    let src = "st-ebpf-a";
    let dst = "st-ebpf-b";

    cleanup_interface(src).await;

    create_veth_pair(src, dst, "ebpf-src", "ebpf-dst").await?;

    let idx_src = get_ifindex(src).await?;
    let idx_dst = get_ifindex(dst).await?;

    // Attach redirect: src ingress → dst egress
    attach_p2p_redirect(src, idx_dst)?;
    // Attach redirect: dst ingress → src egress
    attach_p2p_redirect(dst, idx_src)?;

    // Verify TC clsact qdisc and BPF filter exist on both interfaces
    let output_src = std::process::Command::new("tc")
        .args(["filter", "show", "dev", src, "ingress"])
        .output()
        .expect("tc command failed");
    let tc_src = String::from_utf8_lossy(&output_src.stdout);
    assert!(
        tc_src.contains("bpf"),
        "TC filter on {} should contain 'bpf', got: {}",
        src,
        tc_src
    );

    let output_dst = std::process::Command::new("tc")
        .args(["filter", "show", "dev", dst, "ingress"])
        .output()
        .expect("tc command failed");
    let tc_dst = String::from_utf8_lossy(&output_dst.stdout);
    assert!(
        tc_dst.contains("bpf"),
        "TC filter on {} should contain 'bpf', got: {}",
        dst,
        tc_dst
    );

    // Deleting the interface removes the BPF program automatically
    delete_interface(src).await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_ebpf_redirect_nonexistent_interface_fails() -> Result<()> {
    let result = attach_p2p_redirect("st-noexist99", 1);
    assert!(
        result.is_err(),
        "Attaching eBPF redirect to nonexistent interface should fail"
    );
    Ok(())
}
