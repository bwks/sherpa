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
use std::process::{Child, Command};

use anyhow::Result;

use network::{
    LinkImpairment, apply_netem, attach_p2p_redirect, create_bridge, create_tap,
    create_veth_in_netns, create_veth_pair, delete_interface, enslave_to_bridge,
    find_interfaces_fuzzy, get_ifindex, remove_netem, set_link_down, update_netem,
};

// ============================================================================
// Helper
// ============================================================================

/// Delete an interface, ignoring errors (it may not exist).
async fn cleanup_interface(name: &str) {
    let _ = delete_interface(name).await;
}

/// Returns true if rtnetlink TAP creation is unsupported on this kernel.
///
/// Kernel ≥ 6.8 rejects `ip link add type tun` via rtnetlink with EOPNOTSUPP.
/// The ioctl-based path (`ip tuntap add`) still works; fix is tracked separately.
/// TAP tests call this at entry and skip gracefully when it returns true.
async fn tap_rtnetlink_unsupported() -> bool {
    let probe = "st-tap-probe";
    let _ = delete_interface(probe).await;
    match create_tap(probe, "probe").await {
        Ok(()) => {
            let _ = delete_interface(probe).await;
            false
        }
        Err(e) if format!("{e:?}").contains("Operation not supported") => true,
        Err(_) => false,
    }
}

/// Number of p2p_redirect programs attached to an interface's ingress as legacy TC filters,
/// from `tc filter show dev <iface> ingress`. Programs attached through TCX are not listed
/// there, so a TCX attachment counts as none: it would not survive a sherpad restart, and
/// re-attaching could not replace it.
fn p2p_redirect_filters(iface: &str) -> usize {
    let output = std::process::Command::new("tc")
        .args(["filter", "show", "dev", iface, "ingress"])
        .output()
        .expect("tc command failed");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains(" p2p_redirect "))
        .count()
}

/// Returns the raw output of `ip link show <name>`, or empty string on failure.
fn ip_link_show(name: &str) -> String {
    std::process::Command::new("ip")
        .args(["link", "show", name])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// A process holding a network namespace of its own, standing in for a container. Killed on
/// drop, which removes the namespace and anything in it.
struct NetnsHolder(Child);

impl NetnsHolder {
    fn start() -> Self {
        let child = Command::new("unshare")
            .args(["--net", "sleep", "60"])
            .spawn()
            .expect("unshare command failed");
        // Let unshare enter the new namespace before the test looks it up by pid.
        std::thread::sleep(std::time::Duration::from_millis(200));
        Self(child)
    }

    fn pid(&self) -> u32 {
        self.0.id()
    }

    /// The raw output of `ip link show <name>` inside the namespace, or empty on failure.
    fn ip_link_show(&self, name: &str) -> String {
        Command::new("nsenter")
            .args([
                "-t",
                &self.pid().to_string(),
                "-n",
                "ip",
                "link",
                "show",
                name,
            ])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }
}

impl Drop for NetnsHolder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Whether the flags field of `ip link show` output has `flag`, e.g. `UP` or `PROMISC`.
fn has_flag(ip_link_output: &str, flag: &str) -> bool {
    let Some(start) = ip_link_output.find('<') else {
        return false;
    };
    let Some(end) = ip_link_output[start..].find('>') else {
        return false;
    };
    ip_link_output[start + 1..start + end]
        .split(',')
        .any(|f| f == flag)
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
    if tap_rtnetlink_unsupported().await {
        eprintln!("SKIP: rtnetlink TAP not supported on this kernel — fix deferred");
        return Ok(());
    }

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
    if tap_rtnetlink_unsupported().await {
        eprintln!("SKIP: rtnetlink TAP not supported on this kernel — fix deferred");
        return Ok(());
    }

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
    assert!(
        result.is_err(),
        "Getting ifindex of nonexistent interface should fail"
    );
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

    // Verify p2p_redirect is attached on both interfaces as a TC filter
    assert_eq!(p2p_redirect_filters(src), 1, "p2p_redirect on {src}");
    assert_eq!(p2p_redirect_filters(dst), 1, "p2p_redirect on {dst}");

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

#[tokio::test]
#[ignore]
async fn test_ebpf_redirect_idempotent() -> Result<()> {
    let src = "st-ebpf-id-a";
    let dst = "st-ebpf-id-b";

    cleanup_interface(src).await;

    create_veth_pair(src, dst, "ebpf-id-src", "ebpf-id-dst").await?;

    let idx_src = get_ifindex(src).await?;
    let idx_dst = get_ifindex(dst).await?;

    // Attach once
    attach_p2p_redirect(src, idx_dst)?;

    assert_eq!(
        p2p_redirect_filters(src),
        1,
        "p2p_redirect after first attach"
    );

    // Re-attach with another peer, as redeploy and resume do: the old program is replaced,
    // not left in front of the new one redirecting to the old peer.
    attach_p2p_redirect(src, idx_src)?;
    assert_eq!(p2p_redirect_filters(src), 1, "p2p_redirect after re-attach");

    delete_interface(src).await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_ebpf_redirect_between_taps() -> Result<()> {
    if tap_rtnetlink_unsupported().await {
        eprintln!("SKIP: rtnetlink TAP not supported on this kernel — fix deferred");
        return Ok(());
    }

    let tap_a = "st-ebpf-ta";
    let tap_b = "st-ebpf-tb";

    cleanup_interface(tap_a).await;
    cleanup_interface(tap_b).await;

    create_tap(tap_a, "ebpf-tap-a").await?;
    create_tap(tap_b, "ebpf-tap-b").await?;

    let idx_a = get_ifindex(tap_a).await?;
    let idx_b = get_ifindex(tap_b).await?;

    // Attach bidirectional redirect
    attach_p2p_redirect(tap_a, idx_b)?;
    attach_p2p_redirect(tap_b, idx_a)?;

    // Verify p2p_redirect is attached on both taps as a TC filter
    for name in [tap_a, tap_b] {
        assert_eq!(p2p_redirect_filters(name), 1, "p2p_redirect on {name}");
    }

    delete_interface(tap_a).await?;
    delete_interface(tap_b).await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_ebpf_and_netem_coexist() -> Result<()> {
    let src = "st-ebpf-nm-a";
    let dst = "st-ebpf-nm-b";

    cleanup_interface(src).await;

    create_veth_pair(src, dst, "ebpf-nm-src", "ebpf-nm-dst").await?;

    let idx_src = get_ifindex(src).await?;
    let idx_dst = get_ifindex(dst).await?;

    // Attach eBPF redirect first (clsact qdisc, ingress)
    attach_p2p_redirect(src, idx_dst)?;
    attach_p2p_redirect(dst, idx_src)?;

    // Apply netem on top (root qdisc, egress) — both should coexist
    let impairment = LinkImpairment {
        delay_us: 25000, // 25ms
        jitter_us: 2000, // 2ms
        loss_percent: 0.5,
        reorder_percent: 0.0,
        corrupt_percent: 0.0,
    };

    apply_netem(idx_src as i32, &impairment).await?;
    apply_netem(idx_dst as i32, &impairment).await?;

    // Verify p2p_redirect is still attached after netem
    assert_eq!(p2p_redirect_filters(src), 1, "p2p_redirect after netem");

    let netem_output = std::process::Command::new("tc")
        .args(["qdisc", "show", "dev", src])
        .output()
        .expect("tc command failed");
    let netem_out = String::from_utf8_lossy(&netem_output.stdout);
    assert!(
        netem_out.contains("netem"),
        "netem qdisc should be present, got: {}",
        netem_out
    );
    assert!(
        netem_out.contains("clsact"),
        "clsact qdisc should coexist with netem, got: {}",
        netem_out
    );

    // Verify netem can be removed without affecting eBPF
    remove_netem(idx_src as i32).await?;

    assert_eq!(
        p2p_redirect_filters(src),
        1,
        "p2p_redirect after netem removal"
    );

    delete_interface(src).await?;

    Ok(())
}

// ============================================================================
// set_link_down
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_set_link_down() -> Result<()> {
    let src = "st-down-a";
    let dst = "st-down-b";

    cleanup_interface(src).await;

    create_veth_pair(src, dst, "down-src", "down-dst").await?;

    // Both should be UP after creation
    assert!(
        interface_is_up(src),
        "src should be UP before set_link_down"
    );
    assert!(
        interface_is_up(dst),
        "dst should be UP before set_link_down"
    );

    // Set src DOWN
    set_link_down(src).await?;

    assert!(
        !interface_is_up(src),
        "src should be DOWN after set_link_down"
    );

    // dst loses carrier when peer goes down, but its admin state (UP flag)
    // remains set — verify dst still has UP flag
    assert!(
        interface_is_up(dst),
        "dst admin state should still have UP flag (carrier lost, not admin down)"
    );

    delete_interface(src).await?;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_set_link_down_nonexistent_fails() -> Result<()> {
    let result = set_link_down("st-noexist99").await;
    assert!(
        result.is_err(),
        "Setting down a nonexistent interface should fail"
    );
    Ok(())
}

// ============================================================================
// update_netem
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_update_netem() -> Result<()> {
    let src = "st-netem-up-a";
    let dst = "st-netem-up-b";

    cleanup_interface(src).await;

    create_veth_pair(src, dst, "netem-up-src", "netem-up-dst").await?;

    let idx = get_ifindex(src).await?;

    // Apply initial impairment
    let initial = LinkImpairment {
        delay_us: 10000, // 10ms
        jitter_us: 0,
        loss_percent: 0.0,
        reorder_percent: 0.0,
        corrupt_percent: 0.0,
    };
    apply_netem(idx as i32, &initial).await?;

    // Update to different impairment
    let updated = LinkImpairment {
        delay_us: 100000, // 100ms
        jitter_us: 10000, // 10ms
        loss_percent: 5.0,
        reorder_percent: 0.0,
        corrupt_percent: 0.0,
    };
    update_netem(idx as i32, &updated).await?;

    // Verify netem still present with updated values
    let output = std::process::Command::new("tc")
        .args(["qdisc", "show", "dev", src])
        .output()
        .expect("tc command failed");
    let tc_output = String::from_utf8_lossy(&output.stdout);
    assert!(
        tc_output.contains("netem"),
        "netem should still be present after update, got: {}",
        tc_output
    );
    // 100ms delay should appear in tc output
    assert!(
        tc_output.contains("100ms") || tc_output.contains("100.0ms"),
        "tc output should show ~100ms delay, got: {}",
        tc_output
    );

    delete_interface(src).await?;

    Ok(())
}

// ============================================================================
// Veth created into another network namespace
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_create_veth_in_netns_enabled() -> Result<()> {
    let host = "st-nsv-on";
    cleanup_interface(host).await;
    let holder = NetnsHolder::start();

    create_veth_in_netns(host, "test-nsv-on", "eth1", holder.pid(), true).await?;

    // The far end is in the namespace, already named, up and promiscuous, without anything
    // run inside the namespace.
    let inside = holder.ip_link_show("eth1");
    assert!(has_flag(&inside, "UP"), "eth1 should be up: {inside}");
    assert!(
        has_flag(&inside, "PROMISC"),
        "eth1 should be promiscuous: {inside}"
    );
    assert!(
        has_flag(&inside, "LOWER_UP"),
        "eth1 should have carrier: {inside}"
    );
    assert!(
        inside.contains("mtu 9600"),
        "eth1 should have jumbo MTU: {inside}"
    );

    let outside = ip_link_show(host);
    assert!(has_flag(&outside, "UP"), "host end should be up: {outside}");
    assert!(
        outside.contains("mtu 9600"),
        "host end should have jumbo MTU: {outside}"
    );
    assert!(
        outside.contains("alias test-nsv-on"),
        "host end alias: {outside}"
    );

    delete_interface(host).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_veth_in_netns_disabled() -> Result<()> {
    let host = "st-nsv-off";
    cleanup_interface(host).await;
    let holder = NetnsHolder::start();

    create_veth_in_netns(host, "test-nsv-off", "eth2", holder.pid(), false).await?;

    // A disabled interface is there but down, with no carrier.
    let inside = holder.ip_link_show("eth2");
    assert!(!inside.is_empty(), "eth2 should exist in the namespace");
    assert!(!has_flag(&inside, "UP"), "eth2 should be down: {inside}");
    assert!(
        !has_flag(&inside, "LOWER_UP"),
        "eth2 should have no carrier: {inside}"
    );

    let outside = ip_link_show(host);
    assert!(
        !has_flag(&outside, "UP"),
        "host end should be down: {outside}"
    );
    assert!(
        outside.contains("alias test-nsv-off"),
        "host end alias: {outside}"
    );

    delete_interface(host).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_veth_in_netns_of_missing_pid_fails() -> Result<()> {
    let host = "st-nsv-nopid";
    cleanup_interface(host).await;
    let holder = NetnsHolder::start();
    let pid = holder.pid();
    drop(holder);

    let result = create_veth_in_netns(host, "test-nsv-nopid", "eth1", pid, true).await;

    assert!(result.is_err(), "a pid that is gone should fail");
    assert!(
        ip_link_show(host).is_empty(),
        "no host end should be left behind"
    );
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_veth_in_netns_named_like_a_host_interface() -> Result<()> {
    // Hosts often have their own eth0 and eth1. The name belongs to the namespace it is created
    // in, so a host interface with the same name must neither block nor be changed.
    let host = "st-nsv-clash";
    let clash = "st-nsv-eth1";
    cleanup_interface(host).await;
    cleanup_interface(clash).await;
    create_veth_pair(clash, "st-nsv-eth1p", "clash", "clash-peer").await?;
    set_link_down(clash).await?;
    let holder = NetnsHolder::start();

    create_veth_in_netns(host, "test-nsv-clash", clash, holder.pid(), true).await?;

    let inside = holder.ip_link_show(clash);
    assert!(
        has_flag(&inside, "UP"),
        "{clash} should be up in the namespace: {inside}"
    );
    assert!(
        has_flag(&inside, "PROMISC"),
        "{clash} should be promiscuous: {inside}"
    );
    let outside = ip_link_show(clash);
    assert!(
        !has_flag(&outside, "UP"),
        "host {clash} must be left down: {outside}"
    );
    assert!(
        !has_flag(&outside, "PROMISC"),
        "host {clash} must not be changed: {outside}"
    );

    delete_interface(host).await?;
    delete_interface(clash).await?;
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_create_veth_in_netns_with_host_end_taken_fails() -> Result<()> {
    let host = "st-nsv-taken";
    cleanup_interface(host).await;
    create_veth_pair(host, "st-nsv-takenp", "taken", "taken-peer").await?;
    let holder = NetnsHolder::start();

    let result = create_veth_in_netns(host, "test-nsv-taken", "eth1", holder.pid(), true).await;

    assert!(
        result.is_err(),
        "a host end name already in use should fail"
    );
    assert!(
        holder.ip_link_show("eth1").is_empty(),
        "nothing should be created in the namespace"
    );
    delete_interface(host).await?;
    Ok(())
}
