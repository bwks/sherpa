//! The lab's definition in the database: the manifest as the user wrote it, built from the
//! manifest the client sent and the parts as written it sends beside it, and the one-time
//! import of labs stored before definitions were.
//!
//! `up` stores a lab's definition with its records, and `redeploy` the node it rebuilds (see
//! `db::definition`). The definition holds no file contents and no resolved variable values;
//! those are sent with each `up` or `redeploy` and used to build the node only. A lab stored
//! before definitions has none; at startup sherpad imports it from the lab's saved manifest,
//! once.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::instrument;

use db::Database;
use shared::data::{DbLab, RecordId};
use shared::util::load_saved_manifest;
use topology::validate::validate_provisioners;
use topology::{Manifest, ManifestAsWritten};

/// What the startup import did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Labs whose definition was imported from their saved manifest.
    pub imported: usize,
    /// Labs left without a definition: their saved manifest is missing, unreadable or does not
    /// fit their records. Each is logged.
    pub skipped: usize,
}

/// The parts as written the client sent beside the manifest, if it sent them.
///
/// # Errors
///
/// Fails when they are not parts as written.
pub fn parse_as_written(as_written: Option<Value>) -> Result<Option<ManifestAsWritten>> {
    as_written
        .map(serde_json::from_value)
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

/// Store a definition for every lab that has none, from the manifest saved in its directory
/// under `labs`: `manifest.toml`, or a legacy `manifest.json`. A lab whose manifest is
/// missing, unreadable, or does not fit its records is logged and left without one; nothing is
/// made up for it. The saved files are left in place.
///
/// A saved manifest is the one the client resolved, so only what survives as written is stored
/// (`Manifest::definition` without parts as written): every parameter but `ztp.config`, which
/// holds the file's contents, and `environment_variables`, whose `$VAR`s hold their values. The
/// file contents in it are not stored. It never held provisioners or `provision_concurrency`.
/// Each dropped field is logged.
///
/// # Errors
///
/// Fails when the labs without a definition cannot be listed.
#[instrument(skip(db), fields(labs = %labs.display()))]
pub async fn import_lab_definitions(db: &Database, labs: &Path) -> Result<ImportSummary> {
    let mut summary = ImportSummary::default();
    for lab in db::list_labs_without_definition(db).await? {
        let lab_dir = labs.join(&lab.lab_id);
        match import_lab(db, &lab, &lab_dir).await {
            Ok(()) => {
                tracing::info!(lab_id = %lab.lab_id, "Imported the lab's definition from its saved manifest");
                summary.imported += 1;
            }
            Err(error) => {
                tracing::warn!(
                    lab_id = %lab.lab_id,
                    lab_dir = %lab_dir.display(),
                    error = format!("{error:#}"),
                    "Lab has no stored definition and its saved manifest cannot be imported; it \
                     is left without one"
                );
                summary.skipped += 1;
            }
        }
    }
    Ok(summary)
}

/// Store what survives as written of the manifest saved in `lab_dir` as `lab`'s definition.
async fn import_lab(db: &Database, lab: &DbLab, lab_dir: &Path) -> Result<()> {
    let saved = load_saved_manifest(lab_dir)?;
    let manifest: Manifest =
        serde_json::from_value(saved).context("The saved manifest is not a valid manifest")?;
    for node in &manifest.nodes {
        let mut dropped = Vec::new();
        if node.ztp.config.is_some() {
            dropped.push("ztp.config");
        }
        if node.environment_variables.is_some() {
            dropped.push("environment_variables");
        }
        if !dropped.is_empty() {
            tracing::warn!(
                lab_id = %lab.lab_id,
                node_name = %node.name,
                fields = %dropped.join(", "),
                "The saved manifest holds these fields resolved, not as written, so they are \
                 not imported; a redeploy from the lab's manifest stores them"
            );
        }
    }
    db::store_lab_definition(db, &db::get_lab_id(lab)?, &manifest.definition(None)?)
        .await
        .context("The saved manifest does not fit the lab's records")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use db::{
        DefaultRequest, LabDetails, LabNetworks, SecretKey, apply_schema, create_lab, create_node,
        create_user, open_in_memory, read_lab_definition, upsert_node_image,
    };
    use shared::data::{NodeConfig, NodeModel};

    use super::*;

    const MANIFEST_TOML: &str = r#"
name = "core"
ready_timeout = 300

[ztp_server]
enable = true
username = "ztp"
password = "ztp-Secret-42"

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

[[nodes.text_files_data]]
content = "V2VsY29tZS1Db250ZW50"
dst = "/etc/motd"
user = "root"
group = "root"
permissions = 644

[[nodes.startup_scripts_data]]
filename = "start.sh"
content = "U2NyaXB0LUNvbnRlbnQ="

[[links]]
src = "dev01::eth1"
dst = "dev01::eth2"
"#;

    /// A keyed database holding the lab `lab-0001` with the node `dev01`, and no definition.
    async fn lab() -> (Database, DbLab) {
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
                loopback_network: "127.127.1.0/24".to_owned(),
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
        let image = upsert_node_image(
            &db,
            NodeConfig::get_model(NodeModel::UbuntuLinux),
            DefaultRequest::Keep,
        )
        .await
        .unwrap();
        create_node(&db, "dev01", 1, image.id.unwrap(), lab.id.clone().unwrap())
            .await
            .unwrap();
        (db, lab)
    }

    /// The saved manifest, as the client resolved it.
    fn saved() -> Manifest {
        toml::from_str(MANIFEST_TOML).unwrap()
    }

    /// What the import stores of [`saved`]: its parameters as written, without `ztp.config`
    /// and `environment_variables`, which it holds resolved, and without file contents.
    fn expected() -> Manifest {
        let mut manifest = saved();
        let node = &mut manifest.nodes[0];
        node.ztp.config = None;
        node.environment_variables = None;
        node.text_files_data = None;
        node.startup_scripts_data = None;
        manifest
    }

    #[tokio::test]
    async fn a_saved_toml_manifest_is_imported_once() {
        let (db, lab) = lab().await;
        let labs = tempfile::tempdir().unwrap();
        fs::create_dir(labs.path().join("lab-0001")).unwrap();
        fs::write(labs.path().join("lab-0001/manifest.toml"), MANIFEST_TOML).unwrap();

        let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                imported: 1,
                skipped: 0
            }
        );
        let stored = read_lab_definition(&db, lab.id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(stored, Some(expected()));
        let stored = stored.unwrap();
        assert_eq!(
            stored.nodes[0].text_files.as_ref().unwrap()[0].src,
            "files/motd"
        );
        assert_eq!(
            stored.nodes[0].startup_scripts.as_deref(),
            Some(&["scripts/start.sh".to_owned()][..])
        );
        assert!(labs.path().join("lab-0001/manifest.toml").exists());

        // A second start finds nothing to import.
        let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
        assert_eq!(summary, ImportSummary::default());
    }

    #[tokio::test]
    async fn a_legacy_json_manifest_is_imported() {
        let (db, lab) = lab().await;
        let labs = tempfile::tempdir().unwrap();
        fs::create_dir(labs.path().join("lab-0001")).unwrap();
        fs::write(
            labs.path().join("lab-0001/manifest.json"),
            serde_json::to_string(&saved()).unwrap(),
        )
        .unwrap();

        let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
        assert_eq!(summary.imported, 1);
        let stored = read_lab_definition(&db, lab.id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(stored, Some(expected()));
    }

    #[tokio::test]
    async fn a_missing_manifest_leaves_the_lab_without_a_definition() {
        let (db, lab) = lab().await;
        let labs = tempfile::tempdir().unwrap();

        let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                imported: 0,
                skipped: 1
            }
        );
        let stored = read_lab_definition(&db, lab.id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(stored, None);
    }

    #[tokio::test]
    async fn an_unreadable_or_old_shape_manifest_leaves_the_lab_without_a_definition() {
        let (db, lab) = lab().await;
        let labs = tempfile::tempdir().unwrap();
        let lab_dir = labs.path().join("lab-0001");
        fs::create_dir(&lab_dir).unwrap();
        let manifest = lab_dir.join("manifest.toml");
        for contents in [
            "name = 'core'\nnodes = [\n",
            "name = 'core'\nnodes = [{ name = 'dev01', model = 'ubuntu_linux', cpu_count = 2 }]\n",
        ] {
            fs::write(&manifest, contents).unwrap();
            let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
            assert_eq!(summary.skipped, 1, "{contents}");
        }
        // A file the server cannot open.
        fs::write(&manifest, MANIFEST_TOML).unwrap();
        fs::set_permissions(&manifest, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&manifest).is_err() {
            let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
            assert_eq!(summary.skipped, 1);
        }
        let stored = read_lab_definition(&db, lab.id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(stored, None);
    }

    #[tokio::test]
    async fn a_manifest_that_does_not_fit_the_records_is_not_imported() {
        let (db, lab) = lab().await;
        let labs = tempfile::tempdir().unwrap();
        fs::create_dir(labs.path().join("lab-0001")).unwrap();
        fs::write(
            labs.path().join("lab-0001/manifest.toml"),
            MANIFEST_TOML.replace("dev01", "other"),
        )
        .unwrap();
        let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
        assert_eq!(summary.skipped, 1);
        let stored = read_lab_definition(&db, lab.id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(stored, None);
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
        let parts = parse_as_written(Some(sent)).unwrap().unwrap();
        let stored = definition(&saved(), Some(&parts)).unwrap();
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
        let without = definition(&saved(), None).unwrap();
        assert_eq!(without, expected());

        let zero = serde_json::json!({ "provision_concurrency": 0 });
        let zero = parse_as_written(Some(zero)).unwrap();
        assert!(definition(&saved(), zero.as_ref()).is_err());
        let ghost = serde_json::json!({ "nodes": { "ghost": {} } });
        let ghost = parse_as_written(Some(ghost)).unwrap();
        assert!(definition(&saved(), ghost.as_ref()).is_err());
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
        let parts = parse_as_written(Some(sent)).unwrap();
        db::store_lab_definition(&db, &lab_id, &definition(&saved(), parts.as_ref()).unwrap())
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
