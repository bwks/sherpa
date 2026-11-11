//! How the admin image pages name a node kind. Rust picks the label; the
//! `snippets/kind-badge.html.jinja` partial only gives each kind its colours.

use shared::data::NodeKind;

/// A node kind pill: the label to show, drawn in the kind's colours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindBadge {
    pub label: &'static str,
    pub kind: NodeKind,
}

impl KindBadge {
    /// The pill for a table row, with the short label.
    pub fn short(kind: NodeKind) -> Self {
        Self {
            label: match kind {
                NodeKind::VirtualMachine => "VM",
                NodeKind::Container | NodeKind::Unikernel => kind_label(&kind),
            },
            kind,
        }
    }

    /// The pill for a detail page, with the full label.
    pub fn full(kind: NodeKind) -> Self {
        Self {
            label: kind_label(&kind),
            kind,
        }
    }
}

/// A node kind's name as the web UI shows it.
pub fn kind_label(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::VirtualMachine => "Virtual Machine",
        NodeKind::Container => "Container",
        NodeKind::Unikernel => "Unikernel",
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn every_node_kind_has_a_full_and_a_short_label() {
        let cases = [
            (NodeKind::VirtualMachine, "Virtual Machine", "VM"),
            (NodeKind::Container, "Container", "Container"),
            (NodeKind::Unikernel, "Unikernel", "Unikernel"),
        ];
        assert_eq!(cases.len(), NodeKind::iter().count());
        for (kind, full, short) in cases {
            assert_eq!(kind_label(&kind), full, "{kind:?}");
            assert_eq!(
                KindBadge::full(kind.clone()),
                KindBadge {
                    label: full,
                    kind: kind.clone()
                }
            );
            assert_eq!(
                KindBadge::short(kind.clone()),
                KindBadge { label: short, kind }
            );
        }
    }
}
