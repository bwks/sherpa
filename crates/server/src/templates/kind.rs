//! How the web UI names a node kind. Rust picks the label; the
//! `snippets/kind-badge.html.jinja` partial only gives each kind its colours. The forms that
//! pick a model and a kind read each model's kinds from its [`ModelChoice`].

use shared::data::{NodeKind, NodeModel};
use strum::IntoEnumIterator;

use super::FormChoice;

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
                NodeKind::Container | NodeKind::Unikernel => kind_label(kind),
            },
            kind,
        }
    }

    /// The pill for a detail page, with the full label.
    pub fn full(kind: NodeKind) -> Self {
        Self {
            label: kind_label(kind),
            kind,
        }
    }
}

/// A node kind's name as the web UI shows it.
pub fn kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::VirtualMachine => "Virtual Machine",
        NodeKind::Container => "Container",
        NodeKind::Unikernel => "Unikernel",
    }
}

/// Every kind as a form option, labelled as the web UI names it, none selected. A form's
/// script offers only the kinds of the chosen model.
pub fn kind_choices() -> Vec<FormChoice> {
    NodeKind::iter()
        .map(|kind| FormChoice {
            value: kind.to_string(),
            label: kind_label(kind).to_owned(),
            selected: false,
        })
        .collect()
}

/// A model as a form offers it, with what the form's script needs to offer its kinds: the
/// kinds it takes, comma-separated, and its default kind, empty for a model with none
/// (`generic`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub model: String,
    pub kinds: String,
    pub default_kind: String,
}

impl From<NodeModel> for ModelChoice {
    fn from(model: NodeModel) -> Self {
        Self {
            model: model.to_string(),
            kinds: model
                .kinds()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
            default_kind: model
                .default_kind()
                .map_or_else(String::new, |kind| kind.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
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
            assert_eq!(kind_label(kind), full, "{kind:?}");
            assert_eq!(KindBadge::full(kind), KindBadge { label: full, kind });
            assert_eq!(KindBadge::short(kind), KindBadge { label: short, kind });
        }
    }

    #[test]
    fn a_model_choice_lists_the_models_kinds_and_its_default() {
        assert_eq!(
            ModelChoice::from(NodeModel::AristaEos),
            ModelChoice {
                model: "arista_eos".to_owned(),
                kinds: "virtual_machine,container".to_owned(),
                default_kind: "virtual_machine".to_owned(),
            }
        );
        assert_eq!(
            ModelChoice::from(NodeModel::Generic),
            ModelChoice {
                model: "generic".to_owned(),
                kinds: "virtual_machine,container,unikernel".to_owned(),
                default_kind: String::new(),
            }
        );
        assert_eq!(ModelChoice::from(NodeModel::MongoDb).kinds, "container");
        assert_eq!(
            ModelChoice::from(NodeModel::MongoDb).default_kind,
            "container"
        );
    }

    #[test]
    fn the_kind_choices_are_every_kind_by_its_label() {
        let choices = kind_choices();
        let values: Vec<&str> = choices.iter().map(|choice| choice.value.as_str()).collect();
        let labels: Vec<&str> = choices.iter().map(|choice| choice.label.as_str()).collect();
        assert_eq!(values, ["virtual_machine", "container", "unikernel"]);
        assert_eq!(labels, ["Virtual Machine", "Container", "Unikernel"]);
        assert!(choices.iter().all(|choice| !choice.selected));
    }
}
