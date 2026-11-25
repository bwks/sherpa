/// Integration tests for the network crate.
///
/// These tests require root privileges or the `CAP_NET_ADMIN` capability.
/// Run: sudo -E cargo test -p network -- --ignored --test-threads=1
///
/// eBPF tests additionally require `CAP_BPF` and `CAP_PERFMON`.
///
/// Tests create and destroy their own interfaces.
/// Interface names are prefixed with "st-" (sherpa-test) to avoid collisions.
/// Names are kept short — Linux interface names are limited to 15 characters.
use std::fs;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

use anyhow::Result;

use network::{
    NetemRemoval, NetnsVethState, NetworkError, apply_netem, attach_p2p_redirect,
    container_veth_peer_ifindex, create_bridge, create_veth_in_netns, create_veth_pair,
    delete_interface, disable_tx_checksum_offload, enslave_to_bridge, get_ifindex,
    list_interface_names, remove_netem, set_link_state, sync_netem,
};
use shared::data::LinkImpairment;

// ============================================================================
// Helper
// ============================================================================

/// Delete an interface, ignoring errors (it may not exist).
async fn cleanup_interface(name: &str) {
    let _ = delete_interface(name).await;
}

/// Number of `p2p_redirect` programs attached to an interface's ingress as legacy TC filters,
/// from `tc filter show dev <iface> ingress`. Programs attached through TCX are not listed
/// there, so a TCX attachment counts as none: it would not survive a sherpad restart, and
/// re-attaching could not replace it.
fn p2p_redirect_filters(iface: &str) -> usize {
    let output = Command::new("tc")
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
    Command::new("ip")
        .args(["link", "show", name])
        .output()
        .map_or_default(|o| String::from_utf8_lossy(&o.stdout).into_owned())
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
        thread::sleep(Duration::from_millis(200));
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
            .map_or_default(|o| String::from_utf8_lossy(&o.stdout).into_owned())
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

/// The host's interfaces whose names contain `part`.
async fn interfaces_containing(part: &str) -> Result<Vec<String>> {
    let names = list_interface_names().await?;
    Ok(names
        .into_iter()
        .filter(|name| name.contains(part))
        .collect())
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn private_bridge_005_a_bridge_is_made_up_with_mtu_9600() -> Result<()> {
    let name = "st-br0";
    let alias = "sherpa-test-bridge";

    cleanup_interface(name).await;

    create_bridge(name, alias).await?;

    // Verify bridge exists via fuzzy search
    let found = interfaces_containing(name).await?;
    assert!(
        found.iter().any(|n| n == name),
        "Bridge {name} should exist, found: {found:?}"
    );

    // Cleanup
    delete_interface(name).await?;

    // Verify gone
    let found = interfaces_containing(name).await?;
    assert!(
        !found.iter().any(|n| n == name),
        "Bridge should be gone after deletion"
    );

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
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn private_bridge_011_a_taken_bridge_name_or_a_missing_member_or_bridge_fails() -> Result<()>
{
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
// Enslave to bridge
// ============================================================================

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn private_bridge_006_a_host_end_joins_its_bridge() -> Result<()> {
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

    let shown = ip_link_show(src);
    assert!(
        shown.contains(&format!("master {br}")),
        "{src} should be a port of {br}: {shown}"
    );

    // Cleanup
    delete_interface(src).await?;
    delete_interface(br).await?;

    Ok(())
}

// ============================================================================
// Delete interface
// ============================================================================

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn link_cleanup_005_deleting_an_interface_already_gone_succeeds() -> Result<()> {
    // An interface that is already gone counts as deleted, so a retried destroy succeeds.
    delete_interface("st-noexist99").await?;
    Ok(())
}

// ============================================================================
// TC netem impairment
// ============================================================================

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn link_impair_014_a_new_impairment_replaces_the_old_and_a_clear_leaves_no_netem()
-> Result<()> {
    let src = "st-netem-sync-a";
    let dst = "st-netem-sync-b";

    cleanup_interface(src).await;
    create_veth_pair(src, dst, "netem-sync-src", "netem-sync-dst").await?;
    let idx = get_ifindex(src).await?;
    let tc_show = || {
        let output = Command::new("tc")
            .args(["qdisc", "show", "dev", src])
            .output()
            .expect("tc command failed");
        String::from_utf8_lossy(&output.stdout).to_string()
    };

    let impairment = LinkImpairment {
        delay_us: 10000,
        ..LinkImpairment::default()
    };
    sync_netem(idx, &impairment).await?;
    assert!(tc_show().contains("netem"), "{}", tc_show());
    sync_netem(idx, &LinkImpairment::default()).await?;
    assert!(!tc_show().contains("netem"), "{}", tc_show());
    // Clear again: nothing to remove, not an error.
    sync_netem(idx, &LinkImpairment::default()).await?;

    delete_interface(src).await?;

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

    apply_netem(idx, &impairment).await?;
    assert_eq!(remove_netem(idx).await?, NetemRemoval::Removed);
    // The second removal finds nothing: only that counts as "clear" without a removal.
    assert_eq!(remove_netem(idx).await?, NetemRemoval::NothingToRemove);

    // Verify netem qdisc is gone
    let output = Command::new("tc")
        .args(["qdisc", "show", "dev", src])
        .output()
        .expect("tc command failed");
    let tc_output = String::from_utf8_lossy(&output.stdout);
    assert!(
        !tc_output.contains("netem"),
        "netem should be removed, got: {tc_output}"
    );

    delete_interface(src).await?;

    // An index no interface has holds no qdisc: a removed end has nothing left to remove.
    assert_eq!(
        remove_netem(2_000_000_000).await?,
        NetemRemoval::NothingToRemove
    );
    Ok(())
}

// ============================================================================
// eBPF P2p redirect
// Requires: CAP_NET_ADMIN + CAP_BPF + CAP_PERFMON
// ============================================================================

/// The number of file descriptors this process has open.
fn open_descriptors() -> usize {
    fs::read_dir("/proc/self/fd")
        .expect("lists open descriptors")
        .count()
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn p2p_link_010_attaching_a_link_keeps_no_descriptor_open() -> Result<()> {
    let src = "st-ebpf-fd-a";
    let dst = "st-ebpf-fd-b";

    cleanup_interface(src).await;
    create_veth_pair(src, dst, "ebpf-src", "ebpf-dst").await?;
    let idx_dst = get_ifindex(dst).await?;

    let before = open_descriptors();
    attach_p2p_redirect(src, idx_dst)?;
    // The kernel keeps the program alive through the TC filter, not through sherpad.
    assert_eq!(
        open_descriptors(),
        before,
        "descriptors left open by attach"
    );
    assert_eq!(p2p_redirect_filters(src), 1, "p2p_redirect on {src}");

    delete_interface(src).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn p2p_link_012_a_missing_host_end_is_not_attached_and_the_error_carries_it() -> Result<()> {
    let result = attach_p2p_redirect("st-noexist99", 1);
    assert!(
        result.is_err(),
        "Attaching eBPF redirect to nonexistent interface should fail"
    );

    let error = get_ifindex("st-noexist99").await.unwrap_err();
    let Some(NetworkError::LinkNotFound { name }) = error.downcast_ref::<NetworkError>() else {
        panic!("not a missing link: {error:#}");
    };
    assert_eq!(name, "st-noexist99");
    Ok(())
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn p2p_link_011_attaching_an_attached_link_leaves_one_redirect_on_its_end() -> Result<()> {
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

// ============================================================================
// Veth created into another network namespace
// ============================================================================

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn node_interfaces_006_an_unused_container_interface_is_down_or_up_with_no_carrier()
-> Result<()> {
    let host = "st-nsv-off";
    cleanup_interface(host).await;
    let holder = NetnsHolder::start();

    create_veth_in_netns(
        host,
        "test-nsv-off",
        "eth2",
        holder.pid(),
        NetnsVethState::Disabled,
    )
    .await?;

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

    let host = "st-nsv-dup";
    cleanup_interface(host).await;
    let holder = NetnsHolder::start();

    create_veth_in_netns(
        host,
        "test-nsv-dup",
        "eth3",
        holder.pid(),
        NetnsVethState::DisabledUp,
    )
    .await?;

    // Up in the container, but with no carrier, since the host end is down.
    let inside = holder.ip_link_show("eth3");
    assert!(has_flag(&inside, "UP"), "eth3 should be up: {inside}");
    assert!(
        !has_flag(&inside, "LOWER_UP"),
        "eth3 should have no carrier: {inside}"
    );
    let outside = ip_link_show(host);
    assert!(
        !has_flag(&outside, "UP"),
        "host end should be down: {outside}"
    );

    delete_interface(host).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn p2p_link_015_an_interface_for_a_gone_container_fails_and_leaves_no_host_end() -> Result<()>
{
    let host = "st-nsv-nopid";
    cleanup_interface(host).await;
    let holder = NetnsHolder::start();
    let pid = holder.pid();
    drop(holder);

    let result =
        create_veth_in_netns(host, "test-nsv-nopid", "eth1", pid, NetnsVethState::Enabled).await;

    assert!(result.is_err(), "a pid that is gone should fail");
    assert!(
        ip_link_show(host).is_empty(),
        "no host end should be left behind"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn p2p_link_013_a_container_interface_named_like_a_host_interface_is_made() -> Result<()> {
    // Hosts often have their own eth0 and eth1. The name belongs to the namespace it is created
    // in, so a host interface with the same name must neither block nor be changed.
    let host = "st-nsv-clash";
    let clash = "st-nsv-eth1";
    cleanup_interface(host).await;
    cleanup_interface(clash).await;
    create_veth_pair(clash, "st-nsv-eth1p", "clash", "clash-peer").await?;
    set_link_state(clash, false).await?;
    let holder = NetnsHolder::start();

    create_veth_in_netns(
        host,
        "test-nsv-clash",
        clash,
        holder.pid(),
        NetnsVethState::Enabled,
    )
    .await?;

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
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn link_reattach_027_an_interface_is_made_again_beside_the_removed_containers_old_one()
-> Result<()> {
    let host = "st-nsv-stale";
    cleanup_interface(host).await;
    // The namespace of a removed container can outlive it for minutes, with its veths.
    let removed = NetnsHolder::start();
    let replacement = NetnsHolder::start();
    create_veth_in_netns(
        host,
        "test-nsv-stale",
        "eth1",
        removed.pid(),
        NetnsVethState::Enabled,
    )
    .await?;

    create_veth_in_netns(
        host,
        "test-nsv-stale",
        "eth1",
        replacement.pid(),
        NetnsVethState::Enabled,
    )
    .await?;

    assert!(
        removed.ip_link_show("eth1").is_empty(),
        "the stale pair should be deleted"
    );
    let inside = replacement.ip_link_show("eth1");
    assert!(
        has_flag(&inside, "LOWER_UP"),
        "eth1 should be connected: {inside}"
    );

    delete_interface(host).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn p2p_link_014_a_host_end_name_held_by_another_interface_is_refused_and_left() -> Result<()>
{
    let host = "st-nsv-taken";
    cleanup_interface(host).await;
    create_veth_pair(host, "st-nsv-takenp", "taken", "taken-peer").await?;
    let holder = NetnsHolder::start();

    let result = create_veth_in_netns(
        host,
        "test-nsv-taken",
        "eth1",
        holder.pid(),
        NetnsVethState::Enabled,
    )
    .await;

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

// ============================================================================
// Transmit checksum offload
// ============================================================================

/// The state of feature `feature` in `ethtool -k <iface>`, such as `on` or `off [fixed]`.
fn ethtool_feature(iface: &str, feature: &str) -> String {
    let output = Command::new("ethtool")
        .args(["-k", iface])
        .output()
        .expect("ethtool command failed");
    let prefix = format!("{feature}: ");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::to_owned))
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn veth_checksums_002_turning_offload_off_turns_checksum_and_segmentation_off() -> Result<()>
{
    let host = "st-csum-a";
    let peer = "st-csum-b";
    cleanup_interface(host).await;
    create_veth_pair(host, peer, "test-csum-host", "test-csum-peer").await?;
    assert!(ethtool_feature(host, "tx-checksum-ip-generic").starts_with("on"));

    let ifindex = get_ifindex(host).await?;
    disable_tx_checksum_offload(ifindex)?;
    assert!(ethtool_feature(host, "tx-checksum-ip-generic").starts_with("off"));
    assert!(ethtool_feature(host, "tcp-segmentation-offload").starts_with("off"));
    // Only the end it was asked for.
    assert!(ethtool_feature(peer, "tx-checksum-ip-generic").starts_with("on"));

    // Again, with it off already, changes nothing.
    disable_tx_checksum_offload(ifindex)?;
    assert!(ethtool_feature(host, "tx-checksum-ip-generic").starts_with("off"));

    delete_interface(host).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn veth_checksums_006_turning_offload_off_on_a_missing_interface_fails() -> Result<()> {
    let result = disable_tx_checksum_offload(u32::MAX - 1);
    assert!(result.is_err(), "expected an error, got {result:?}");
    Ok(())
}

#[tokio::test]
#[ignore = "requires root or CAP_NET_ADMIN"]
async fn veth_checksums_007_a_container_interfaces_host_end_is_found_from_its_namespace()
-> Result<()> {
    let container = NetnsHolder::start();
    let host = "st-peer-host";
    cleanup_interface(host).await;
    create_veth_in_netns(
        host,
        "test-peer-host",
        "eth0",
        container.pid(),
        NetnsVethState::Enabled,
    )
    .await?;

    let peer = container_veth_peer_ifindex(container.pid(), "eth0").await;
    let expected = get_ifindex(host).await?;
    delete_interface(host).await?;
    assert_eq!(peer?, expected);
    assert!(
        container_veth_peer_ifindex(container.pid(), "eth9")
            .await
            .is_err()
    );
    Ok(())
}
