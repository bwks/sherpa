//! Which kinds of node take which settings: the one per-kind model that the manifest's node
//! checks (`topology::validate::check_node_kind_settings`) and the node image's groups
//! (`NodeConfig`) share.
//!
//! A setting is listed with the kinds that take it. A node or an image that sets one its kind
//! does not take is refused, naming the setting. Each kind has one group of its own, named as
//! the manifest names it: `vm`, `container` and `unikernel`.

use super::node::NodeKind;

/// A setting's key, whether it is set, and the kinds of node that take it.
pub type KindSetting<'a> = (&'a str, bool, &'static [NodeKind]);

/// Every kind of node.
pub const ALL: &[NodeKind] = &[
    NodeKind::VirtualMachine,
    NodeKind::Container,
    NodeKind::Unikernel,
];
pub const VM: &[NodeKind] = &[NodeKind::VirtualMachine];
pub const CONTAINER: &[NodeKind] = &[NodeKind::Container];
pub const UNIKERNEL: &[NodeKind] = &[NodeKind::Unikernel];
pub const VM_AND_CONTAINER: &[NodeKind] = &[NodeKind::VirtualMachine, NodeKind::Container];
pub const VM_AND_UNIKERNEL: &[NodeKind] = &[NodeKind::VirtualMachine, NodeKind::Unikernel];

/// The group only a virtual machine takes.
pub const VM_GROUP: &str = "vm";
/// The group only a container takes.
pub const CONTAINER_GROUP: &str = "container";
/// The group only a unikernel takes.
pub const UNIKERNEL_GROUP: &str = "unikernel";

/// The name of `kind`'s own group.
#[must_use]
pub fn group_of(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::VirtualMachine => VM_GROUP,
        NodeKind::Container => CONTAINER_GROUP,
        NodeKind::Unikernel => UNIKERNEL_GROUP,
    }
}

/// The first of `settings` that is set but that `kind` does not take, if any.
#[must_use]
pub fn first_not_taken<'a, 'b>(
    kind: NodeKind,
    settings: &'b [KindSetting<'a>],
) -> Option<&'b KindSetting<'a>> {
    settings
        .iter()
        .find(|(_, set, kinds)| *set && !kinds.contains(&kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_has_its_own_group() {
        assert_eq!(group_of(NodeKind::VirtualMachine), "vm");
        assert_eq!(group_of(NodeKind::Container), "container");
        assert_eq!(group_of(NodeKind::Unikernel), "unikernel");
    }

    #[test]
    fn the_first_setting_a_kind_does_not_take_is_named() {
        let settings = [
            ("memory", true, ALL),
            ("vm", false, VM),
            ("container", true, CONTAINER),
            ("unikernel", true, UNIKERNEL),
        ];
        let refused = first_not_taken(NodeKind::VirtualMachine, &settings);
        assert_eq!(refused.map(|(key, ..)| *key), Some("container"));
        let refused = first_not_taken(NodeKind::Unikernel, &settings);
        assert_eq!(refused.map(|(key, ..)| *key), Some("container"));
    }

    #[test]
    fn a_setting_left_unset_is_never_refused() {
        let settings = [("vm", false, VM), ("memory", true, ALL)];
        assert!(first_not_taken(NodeKind::Container, &settings).is_none());
    }
}
