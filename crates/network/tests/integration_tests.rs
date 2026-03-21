/// Integration tests for the network crate.
///
/// These tests require root privileges or CAP_NET_ADMIN capability.
/// Run: sudo -E cargo test -p network -- --ignored --test-threads=1
///
/// Tests create and destroy their own interfaces.
/// Interface names are prefixed with "st-" (sherpa-test) to avoid collisions.
/// Names are kept short — Linux interface names are limited to 15 characters.
use anyhow::Result;

use network::{
    create_bridge, create_veth_pair, delete_interface, enslave_to_bridge, find_interfaces_fuzzy,
};

// ============================================================================
// Helper
// ============================================================================

/// Delete an interface, ignoring errors (it may not exist).
async fn cleanup_interface(name: &str) {
    let _ = delete_interface(name).await;
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
