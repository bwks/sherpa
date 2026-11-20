use anyhow::{Result, bail};

use crate::Node;

/// Rejects the removed per-node `ssh` group. Each node takes the SSH keys on the lab owner's
/// account, so a manifest that still sets the group, in any form, is told to move its keys to
/// the account rather than having them silently ignored.
///
/// # Errors
///
/// Fails when a node sets `ssh`.
pub fn check_no_ssh_group(nodes: &[Node]) -> Result<()> {
    if let Some(node) = nodes.iter().find(|node| node.ssh.is_some()) {
        bail!(
            "Manifest - node: '{}' sets the `ssh` group, which has been removed: every node takes \
             the SSH keys on your account. Delete `ssh` from the node and add the keys to your \
             account with `sherpa server user ssh-key add` or on the Profile page",
            node.name
        );
    }
    Ok(())
}
