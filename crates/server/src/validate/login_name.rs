//! The login names each node model takes for the lab owner's account.
//!
//! A Sherpa username is 3 to 32 of `a-z 0-9 _ -`, starting with a letter or `_`
//! (`shared::validators::validate_guest_username`). Most models take every such name. The ones
//! below take fewer, and a name they cannot take is refused before anything is created, never
//! shortened or changed: a changed name would make the SSH config, the docs and what the person
//! types disagree. Each limit is from the vendor's documentation:
//!
//! - Windows: at most 20 characters (Microsoft, `New-LocalUser`).
//! - Cisco NX-OS: at most 28 characters, starting with a letter or digit (Nexus 9000 security
//!   configuration guide, "Configuring User Accounts and RBAC").
//! - Arista EOS: starting with a letter or digit (EOS User Manual, "User Security").
//! - PAN-OS: at most 31 characters (PAN-OS web interface help, Device > Administrators).
//! - OpenBSD: at most 31 characters (`adduser(8)`).

use anyhow::{Result, bail};

use shared::data::{ModelKind, NodeModel};

/// What a model takes beyond the Sherpa username rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LoginNameRule {
    /// The most characters a login name has.
    max_length: usize,
    /// Whether the name must start with a letter or a digit, so not with `_`.
    alphanumeric_first: bool,
}

/// The longest Sherpa username, which every other model takes.
const SHERPA_MAX_LENGTH: usize = 32;

/// The login names `model` takes.
fn login_name_rule(model: NodeModel) -> LoginNameRule {
    let (max_length, alphanumeric_first) = match model {
        NodeModel::WindowsServer | NodeModel::DevboxWindows => (20, false),
        NodeModel::CiscoNexus9300v => (28, true),
        NodeModel::AristaEos => (SHERPA_MAX_LENGTH, true),
        NodeModel::PaloaltoPanos | NodeModel::OpenBsd => (31, false),
        _ => (SHERPA_MAX_LENGTH, false),
    };
    LoginNameRule {
        max_length,
        alphanumeric_first,
    }
}

/// Check that node `node_name`, of `model_kind`, can take `username` as its login. The rule is
/// the model's, whatever its kind; the message names the kind of a model with several.
///
/// # Errors
///
/// Fails when the name is longer than the model takes, or starts with a character it does not
/// take, naming the node, the model and the limit.
pub fn validate_login_name(model_kind: ModelKind, node_name: &str, username: &str) -> Result<()> {
    let rule = login_name_rule(model_kind.model());
    let length = username.chars().count();
    if length > rule.max_length {
        bail!(
            "Node '{node_name}' ({model_kind}) takes login names of at most {} characters; \
             '{username}' has {length}. Start this lab from an account with a shorter name.",
            rule.max_length
        );
    }
    if rule.alphanumeric_first
        && !username
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
    {
        bail!(
            "Node '{node_name}' ({model_kind}) takes login names that start with a letter or a digit; \
             '{username}' does not. Start this lab from an account whose name does."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeKind;

    /// A name of `length` characters, starting with `first`.
    fn name(first: char, length: usize) -> String {
        let mut name = String::from(first);
        name.push_str(&"a".repeat(length - 1));
        name
    }

    #[test]
    fn each_model_takes_names_up_to_its_limit_and_refuses_longer_ones() {
        for (model, limit) in [
            (NodeModel::WindowsServer, 20),
            (NodeModel::CiscoNexus9300v, 28),
            (NodeModel::PaloaltoPanos, 31),
            (NodeModel::OpenBsd, 31),
            (NodeModel::DevboxWindows, 20),
        ] {
            assert!(
                validate_login_name(ModelKind::default_for(model), "n1", &name('a', limit)).is_ok()
            );
            let error =
                validate_login_name(ModelKind::default_for(model), "n1", &name('a', limit + 1))
                    .unwrap_err()
                    .to_string();
            assert!(
                error.contains(&format!(
                    "Node 'n1' ({model}) takes login names of at most {limit} characters"
                )),
                "{error}"
            );
            assert!(error.contains(&format!("has {}", limit + 1)), "{error}");
        }
    }

    #[test]
    fn nx_os_and_eos_refuse_a_leading_underscore() {
        for model in [NodeModel::CiscoNexus9300v, NodeModel::AristaEos] {
            let error = validate_login_name(ModelKind::default_for(model), "sw1", "_svc")
                .unwrap_err()
                .to_string();
            assert!(error.contains("start with a letter or a digit"), "{error}");
            assert!(error.contains("'_svc'"), "{error}");
            assert!(validate_login_name(ModelKind::default_for(model), "sw1", "svc_1").is_ok());
        }
    }

    #[test]
    fn both_kinds_of_eos_take_its_rule_and_the_message_names_the_kind() {
        for kind in [NodeKind::VirtualMachine, NodeKind::Container] {
            let model_kind = ModelKind::new(NodeModel::AristaEos, Some(kind)).unwrap();
            let error = validate_login_name(model_kind, "sw1", "_svc")
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!(
                    "Node 'sw1' (arista_eos, {kind}) takes login names"
                )),
                "{error}"
            );
        }
    }

    #[test]
    fn other_models_take_every_sherpa_username() {
        for model in [
            NodeModel::UbuntuLinux,
            NodeModel::FreeBsd,
            NodeModel::JuniperSrx,
            NodeModel::CiscoIosv,
            NodeModel::NokiaSrlinux,
            NodeModel::PaloaltoPanos,
            NodeModel::WindowsServer,
        ] {
            assert!(validate_login_name(ModelKind::default_for(model), "n1", "_svc").is_ok());
        }
        for model in [NodeModel::UbuntuLinux, NodeModel::JuniperSrx] {
            assert!(
                validate_login_name(ModelKind::default_for(model), "n1", &name('a', 32)).is_ok()
            );
        }
    }
}
