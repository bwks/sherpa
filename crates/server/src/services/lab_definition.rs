//! The lab's definition in the database: the manifest as the user wrote it, built from the
//! manifest the client sent and the parts as written it sends beside it; and what the server
//! reads of it.
//!
//! `up` stores a lab's definition with its records, and `redeploy` the node it rebuilds (see
//! `db::definition`). The definition holds no file contents and no resolved variable values;
//! those are sent with each `up` or `redeploy` and used to build the node only.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::instrument;

use db::Database;
use shared::data::RecordId;
use topology::validate::validate_provisioners;
use topology::{Manifest, ManifestAsWritten, NodeFileContents};

use crate::api::request_parse::parse_value;
use crate::services::error::ServiceError;
use crate::validate::check_web_redeployable;

/// The manifest the client sent, with the file contents it sent beside it put into its nodes.
///
/// # Errors
///
/// Fails when either is not what it should be, or the file contents name a node the manifest
/// does not have.
pub fn parse_manifest(manifest: &Value, file_contents: Option<&Value>) -> Result<Manifest> {
    let mut manifest: Manifest = parse_value(manifest).context("Failed to deserialize manifest")?;
    let file_contents: Option<BTreeMap<String, NodeFileContents>> = file_contents
        .map(parse_value)
        .transpose()
        .context("Failed to deserialize the file contents")?;
    manifest.put_file_contents(file_contents.unwrap_or_default())?;
    Ok(manifest)
}

/// The parts as written the client sent beside the manifest, if it sent them.
///
/// # Errors
///
/// Fails when they are not parts as written.
pub fn parse_as_written(as_written: Option<&Value>) -> Result<Option<ManifestAsWritten>> {
    as_written
        .map(parse_value)
        .transpose()
        .context("Failed to deserialize the manifest as written")
}

/// The definition to store for `manifest`, as the client sent it: the manifest as written, with
/// `as_written`'s parts and no file contents (`Manifest::definition`), its provisioners checked
/// as the client checks them.
///
/// # Errors
///
/// Fails when `as_written` names a node `manifest` does not have, or the provisioners fail
/// their checks.
pub fn definition(manifest: &Manifest, as_written: Option<&ManifestAsWritten>) -> Result<Manifest> {
    let definition = manifest.definition(as_written)?;
    validate_provisioners(&definition)?;
    Ok(definition)
}

/// The parts as written of node `node_name` in the lab `lab_id`'s stored definition, with the
/// lab's `provision_concurrency`: what a redeploy that did not send them (from the web UI or an
/// older client) keeps. `None` when the lab has no stored definition.
///
/// # Errors
///
/// Fails when the stored definition cannot be read.
pub async fn stored_as_written(
    db: &Database,
    lab_id: &RecordId,
    node_name: &str,
) -> Result<Option<ManifestAsWritten>> {
    let Some(stored) = db::read_lab_definition(db, lab_id).await? else {
        return Ok(None);
    };
    let mut parts = stored.as_written();
    parts.nodes.retain(|name, _| name == node_name);
    Ok(Some(parts))
}

/// The stored definition of the lab `lab_id` (record `lab_record_id`), for the readers that
/// fall back to defaults without one: resume's `ready_timeout`, and a container's data
/// interface count when its link ends are made again. `None`, logged at warn, when the lab has
/// no stored definition or it cannot
/// be read; the readers then use the defaults.
#[instrument(skip(db, lab_record_id), fields(%lab_id), level = "debug")]
pub async fn stored_definition(
    db: &Database,
    lab_record_id: &RecordId,
    lab_id: &str,
) -> Option<Manifest> {
    match db::read_lab_definition(db, lab_record_id).await {
        Ok(Some(definition)) => Some(definition),
        Ok(None) => {
            tracing::warn!(%lab_id, "Lab has no stored definition; using the defaults");
            None
        }
        Err(error) => {
            tracing::warn!(
                %lab_id,
                error = format!("{error:#}"),
                "Failed to read the lab's stored definition; using the defaults"
            );
            None
        }
    }
}

/// The manifest the web UI redeploys node `node_name` of the lab `lab_record_id` from: the lab's
/// stored definition, narrowed to that node, as the redeploy rebuilds only it and stores only
/// its definition and the lab-level settings. The other nodes are left out, so their files do
/// not refuse the redeploy.
///
/// # Errors
///
/// Fails with a [`ServiceError::Conflict`] when the lab has no stored definition, the node is
/// not in it, or it needs what the server does not keep (`check_web_redeployable`); and when
/// the definition cannot be read.
#[instrument(skip(db, lab_record_id), fields(%node_name), level = "debug")]
pub async fn web_redeploy_manifest(
    db: &Database,
    lab_record_id: &RecordId,
    node_name: &str,
) -> Result<Manifest> {
    let Some(mut definition) = db::read_lab_definition(db, lab_record_id).await? else {
        return Err(ServiceError::Conflict(format!(
            "Lab has no stored definition, so node '{node_name}' cannot be redeployed from the \
             web UI. Run `sherpa redeploy -n {node_name}` from the lab folder"
        ))
        .into());
    };
    let node = definition
        .nodes
        .iter()
        .find(|node| node.name == node_name)
        .ok_or_else(|| {
            ServiceError::Conflict(format!(
                "Node '{node_name}' is not in the lab's stored definition"
            ))
        })?;
    check_web_redeployable(node)?;
    definition.nodes.retain(|node| node.name == node_name);
    Ok(definition)
}

#[cfg(test)]
pub(crate) mod tests {
    use db::{
        DefaultRequest, LabDetails, LabNetworks, SecretKey, apply_schema, create_lab, create_node,
        create_user, open_in_memory, upsert_node_image,
    };
    use shared::data::{DbLab, ModelKind, NodeModel};
    use topology::{StartupScript, TextFileData};

    use crate::services::error::ServiceError;

    use super::*;

    const MANIFEST_TOML: &str = r#"
name = "core"
ready_timeout = 300

[[nodes]]
name = "dev01"
model = "ubuntu_linux"
memory = 2048
environment_variables = ["MODE=lab", "TOKEN=resolved-Secret-5"]
startup_scripts = ["scripts/start.sh"]
ztp = { config = "I2Nsb3VkLWNvbmZpZw==" }

[[nodes.text_files]]
src = "files/motd"
dst = "/etc/motd"
user = "root"
group = "root"
permissions = 644

[[links]]
src = "dev01::eth1"
dst = "dev01::eth2"
"#;

    /// A keyed database holding the lab `lab-0001` with the node `dev01`, and no definition.
    async fn lab() -> (Database, DbLab) {
        lab_with(&[("dev01", NodeModel::UbuntuLinux)]).await
    }

    /// A keyed database holding the lab `lab-0001` with these nodes, each of its model's
    /// default image, and no definition.
    pub(crate) async fn lab_with(nodes: &[(&str, NodeModel)]) -> (Database, DbLab) {
        let db = open_in_memory()
            .await
            .unwrap()
            .with_secret_key(SecretKey::generate());
        apply_schema(&db).await.unwrap();
        let user = create_user(&db, "alice".to_owned(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        let lab = create_lab(
            &db,
            "core",
            "lab-0001",
            &user,
            &LabNetworks {
                management_network: "172.31.1.0/24".to_owned(),
                gateway_ipv4: "172.31.1.1".to_owned(),
                router_ipv4: "172.31.1.2".to_owned(),
                management_network_v6: None,
                loopback_network_v6: None,
                gateway_ipv6: None,
                router_ipv6: None,
            },
            &LabDetails::default(),
        )
        .await
        .unwrap();
        for (index, (name, model)) in (1..).zip(nodes) {
            let image = upsert_node_image(
                &db,
                ModelKind::default_for(*model).node_config(),
                DefaultRequest::Keep,
            )
            .await
            .unwrap();
            create_node(&db, name, index, image.id.unwrap(), lab.id.clone().unwrap())
                .await
                .unwrap();
        }
        (db, lab)
    }

    /// Store `manifest`, a manifest in TOML as the user wrote it, as `lab`'s definition.
    pub(crate) async fn store(db: &Database, lab: &DbLab, manifest: &str) {
        let manifest: Manifest = toml::from_str(manifest).unwrap();
        db::store_lab_definition(db, lab.id.as_ref().unwrap(), &manifest)
            .await
            .unwrap();
    }

    /// A manifest as the client resolved and sent it.
    fn resolved() -> Manifest {
        let mut manifest: Manifest = toml::from_str(MANIFEST_TOML).unwrap();
        let files = NodeFileContents {
            text_files: Some(vec![TextFileData {
                content: "V2VsY29tZS1Db250ZW50".to_owned(),
                dst: "/etc/motd".to_owned(),
                user: "root".to_owned(),
                group: "root".to_owned(),
                permissions: 644,
            }]),
            systemd_units: None,
            startup_scripts: Some(vec![StartupScript {
                filename: "start.sh".to_owned(),
                content: "U2NyaXB0LUNvbnRlbnQ=".to_owned(),
            }]),
        };
        manifest
            .put_file_contents(BTreeMap::from([("dev01".to_owned(), files)]))
            .unwrap();
        manifest
    }

    /// The definition of [`resolved`] sent without parts as written: its parameters as written,
    /// without `ztp.config` and `environment_variables`, which it holds resolved, and without
    /// file contents.
    fn without_parts() -> Manifest {
        let mut manifest = resolved();
        let node = &mut manifest.nodes[0];
        node.ztp.config = None;
        node.environment_variables = None;
        node.text_files_data = None;
        node.startup_scripts_data = None;
        manifest
    }

    #[tokio::test]
    async fn the_readers_get_the_stored_definition_or_none() {
        let (db, lab) = lab().await;
        let lab_id = lab.id.clone().unwrap();
        assert_eq!(stored_definition(&db, &lab_id, "lab-0001").await, None);
        store(&db, &lab, &toml::to_string(&without_parts()).unwrap()).await;
        assert_eq!(
            stored_definition(&db, &lab_id, "lab-0001").await,
            Some(without_parts())
        );
    }

    #[tokio::test]
    async fn the_web_redeploys_a_node_from_its_stored_definition_alone() {
        let (db, lab) = lab_with(&[
            ("plain01", NodeModel::UbuntuLinux),
            ("files01", NodeModel::UbuntuLinux),
        ])
        .await;
        let lab_id = lab.id.clone().unwrap();
        let refusal = web_redeploy_manifest(&db, &lab_id, "plain01")
            .await
            .unwrap_err();
        assert_eq!(
            ServiceError::find(&refusal),
            Some(&ServiceError::Conflict(
                "Lab has no stored definition, so node 'plain01' cannot be redeployed from the \
                 web UI. Run `sherpa redeploy -n plain01` from the lab folder"
                    .to_owned()
            ))
        );

        store(
            &db,
            &lab,
            r#"
name = "core"
ready_timeout = 240

[[nodes]]
name = "plain01"
model = "ubuntu_linux"
memory = 2048

[[nodes]]
name = "files01"
model = "ubuntu_linux"
startup_scripts = ["scripts/start.sh"]
ztp = { config = "configs/files01.yaml" }
"#,
        )
        .await;
        // The other node's files do not refuse it: it is left out.
        let manifest = web_redeploy_manifest(&db, &lab_id, "plain01")
            .await
            .unwrap();
        assert_eq!(manifest.ready_timeout, Some(240));
        assert_eq!(manifest.nodes.len(), 1);
        assert_eq!(manifest.nodes[0].name, "plain01");
        assert_eq!(manifest.nodes[0].memory, Some(2048));

        let refusal = web_redeploy_manifest(&db, &lab_id, "files01")
            .await
            .unwrap_err();
        assert_eq!(
            ServiceError::find(&refusal),
            Some(&ServiceError::NotWebRedeployable {
                node: "files01".to_owned(),
                missing: vec!["ztp.config", "startup_scripts"],
            })
        );
        let absent = web_redeploy_manifest(&db, &lab_id, "ghost")
            .await
            .unwrap_err();
        assert_eq!(
            ServiceError::find(&absent),
            Some(&ServiceError::Conflict(
                "Node 'ghost' is not in the lab's stored definition".to_owned()
            ))
        );
    }

    #[test]
    fn the_definition_takes_the_parts_as_written_and_checks_provisioners() {
        let sent = serde_json::json!({
            "provision_concurrency": 2,
            "nodes": { "dev01": {
                "ztp_config": "configs/dev01.yaml",
                "environment_variables": ["MODE=lab", "TOKEN=$LAB_TOKEN"],
                "provisioners": [{ "type": "ssh", "scripts": ["./setup.sh"] }],
            } },
        });
        let parts = parse_as_written(Some(&sent)).unwrap().unwrap();
        let stored = definition(&resolved(), Some(&parts)).unwrap();
        let node = &stored.nodes[0];
        assert_eq!(node.ztp.config.as_deref(), Some("configs/dev01.yaml"));
        assert_eq!(
            node.environment_variables.as_deref(),
            Some(&["MODE=lab".to_owned(), "TOKEN=$LAB_TOKEN".to_owned()][..])
        );
        assert_eq!(stored.provision_concurrency, Some(2));
        assert!(!stored.has_file_contents());
        let text = serde_json::to_string(&stored).unwrap();
        for resolved in [
            "resolved-Secret-5",
            "I2Nsb3Vk",
            "V2VsY29tZS1Db250ZW50",
            "U2NyaXB0",
        ] {
            assert!(!text.contains(resolved), "{resolved}: {text}");
        }

        assert_eq!(parse_as_written(None).unwrap(), None);
        let without = definition(&resolved(), None).unwrap();
        assert_eq!(without, without_parts());

        let zero = serde_json::json!({ "provision_concurrency": 0 });
        let zero = parse_as_written(Some(&zero)).unwrap();
        assert!(definition(&resolved(), zero.as_ref()).is_err());
        let ghost = serde_json::json!({ "nodes": { "ghost": {} } });
        let ghost = parse_as_written(Some(&ghost)).unwrap();
        assert!(definition(&resolved(), ghost.as_ref()).is_err());
    }

    #[tokio::test]
    async fn a_redeploy_without_parts_as_written_keeps_the_stored_ones() {
        let (db, lab) = lab().await;
        let lab_id = lab.id.clone().unwrap();
        assert_eq!(
            stored_as_written(&db, &lab_id, "dev01").await.unwrap(),
            None
        );
        let sent = serde_json::json!({
            "provision_concurrency": 3,
            "nodes": { "dev01": { "ztp_config": "configs/dev01.yaml" } },
        });
        let parts = parse_as_written(Some(&sent)).unwrap();
        db::store_lab_definition(
            &db,
            &lab_id,
            &definition(&resolved(), parts.as_ref()).unwrap(),
        )
        .await
        .unwrap();

        let kept = stored_as_written(&db, &lab_id, "dev01")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kept.provision_concurrency, Some(3));
        assert_eq!(
            kept.nodes["dev01"].ztp_config.as_deref(),
            Some("configs/dev01.yaml")
        );
        assert!(
            stored_as_written(&db, &lab_id, "other")
                .await
                .unwrap()
                .unwrap()
                .nodes
                .is_empty()
        );
    }
}
