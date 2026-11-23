use anyhow::{Result, bail, ensure};
use shared::data::NodeKind;
use shared::konst::TAILSCALE_GATEWAY_NAME;
use tracing::instrument;

use crate::Manifest;

/// Validate configuration without resolving secrets or contacting Tailscale.
///
/// # Errors
///
/// Fails when an enabled tailnet does not name a valid environment variable for its key, or a
/// container node's name would give it the gateway's container name.
#[instrument(skip(manifest), level = "debug")]
pub fn validate_tailscale(manifest: &Manifest) -> Result<()> {
    let Some(config) = &manifest.tailscale else {
        return Ok(());
    };
    if !config.enabled {
        return Ok(());
    }
    let name = config.auth_key_env.as_deref().unwrap_or_default();
    ensure!(
        valid_env_name(name),
        "tailscale.auth_key_env must name an environment variable (letters, digits, underscores; not starting with a digit)"
    );
    for node in &manifest.nodes {
        if node.name == TAILSCALE_GATEWAY_NAME
            && node.kind.or(node.model.default_kind()) == Some(NodeKind::Container)
        {
            bail!(
                "Manifest - node: '{}' is a container node with the Tailscale gateway's name; rename it, or disable [tailscale]",
                node.name
            );
        }
    }
    Ok(())
}

fn valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, TailscaleConfig};
    use shared::data::NodeModel;

    fn enabled(auth_key_env: Option<&str>) -> Manifest {
        Manifest {
            tailscale: Some(TailscaleConfig {
                enabled: true,
                auth_key_env: auth_key_env.map(str::to_owned),
            }),
            ..Manifest::default()
        }
    }

    #[test]
    fn lab_settings_004_an_enabled_tailnet_names_the_variable_that_holds_its_key() {
        let refused = [
            None,
            Some(""),
            Some("1KEY"),
            Some("$KEY"),
            Some("key-name"),
            Some("tskey-auth-secret"),
            Some("KEY=value"),
        ];
        for name in refused {
            let error = validate_tailscale(&enabled(name)).unwrap_err().to_string();
            assert!(
                error.contains("tailscale.auth_key_env must name an environment variable"),
                "{name:?}: {error}"
            );
        }
        validate_tailscale(&enabled(Some("SHERPA_TAILSCALE_KEY"))).unwrap();
    }

    #[test]
    fn node_naming_005_a_container_node_with_the_gateway_name_is_refused_in_a_tailnet_lab() {
        let node = |name: &str, model: NodeModel, kind: Option<NodeKind>| Node {
            name: name.to_owned(),
            model,
            kind,
            ..Node::default()
        };
        let lab = |nodes: Vec<Node>, enabled: bool| Manifest {
            nodes,
            tailscale: Some(TailscaleConfig {
                enabled,
                auth_key_env: Some("SHERPA_TAILSCALE_KEY".to_owned()),
            }),
            ..Manifest::default()
        };
        let clashing = [
            node("sherpa-tailnet", NodeModel::FrrLinux, None),
            node(
                "sherpa-tailnet",
                NodeModel::Generic,
                Some(NodeKind::Container),
            ),
        ];
        for clash in clashing {
            let error = validate_tailscale(&lab(vec![clash], true))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("'sherpa-tailnet'") && error.contains("gateway"),
                "{error}"
            );
        }
        let taken = [
            lab(
                vec![node("sherpa-tailnet-app", NodeModel::FrrLinux, None)],
                true,
            ),
            lab(
                vec![node("sherpa-tailnet", NodeModel::AlpineLinux, None)],
                true,
            ),
            lab(
                vec![node("sherpa-tailnet", NodeModel::FrrLinux, None)],
                false,
            ),
        ];
        for manifest in taken {
            validate_tailscale(&manifest).unwrap();
        }
    }
}
