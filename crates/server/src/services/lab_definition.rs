//! The lab's definition in the database: the manifest as the user wrote it, built from the
//! manifest the client sent and the parts as written it sends beside it; what the server reads
//! of it; and the one-time import of labs stored before definitions were.
//!
//! `up` stores a lab's definition with its records, and `redeploy` the node it rebuilds (see
//! `db::definition`). The definition holds no file contents and no resolved variable values;
//! those are sent with each `up` or `redeploy` and used to build the node only. A lab stored
//! before definitions has none; at startup sherpad imports it from the manifest an older
//! release saved in the lab's directory, once, and deletes the saved manifest.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::instrument;

use db::Database;
use shared::data::{DbLab, RecordId};
use topology::validate::validate_provisioners;
use topology::{Manifest, ManifestAsWritten};

use crate::services::error::ServiceError;
use crate::validate::check_web_redeployable;

/// The manifest an older release saved in each lab's directory.
const SAVED_MANIFEST_FILE: &str = "manifest.toml";
/// The manifest releases before the TOML one saved, as JSON.
const LEGACY_SAVED_MANIFEST_FILE: &str = "manifest.json";

/// What the startup import did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Labs whose definition was imported from their saved manifest.
    pub imported: usize,
    /// Labs left without a definition: their saved manifest is missing, unreadable or does not
    /// fit their records. Each is logged.
    pub skipped: usize,
    /// Saved manifest files deleted.
    pub deleted: usize,
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

/// The stored definition of the lab `lab_id` (record `lab_record_id`), for the readers that
/// fall back to defaults without one: resume's `ready_timeout`, and a container's data
/// interface count when its link ends are made again. `None`, logged at warn, when the lab has
/// no stored definition (one whose saved manifest could not be imported) or it cannot be read;
/// the readers then use the defaults, as they did for a missing saved manifest.
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
/// not in it, or the node needs what the server does not keep (`check_web_redeployable`); and
/// when the definition cannot be read.
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
    check_web_redeployable(node).map_err(|error| ServiceError::Conflict(error.to_string()))?;
    definition.nodes.retain(|node| node.name == node_name);
    Ok(definition)
}

/// Store a definition for every lab that has none, from the manifest saved in its directory
/// under `labs`: `manifest.toml`, or a legacy `manifest.json`. A lab whose manifest is
/// missing, unreadable, or does not fit its records is logged and left without one; nothing is
/// made up for it. Then delete every lab's saved manifests ([`delete_saved_manifests`]): the
/// definition is the lab's record of its manifest, and nothing reads the files.
///
/// A saved manifest is the one the client resolved, so only what survives as written is stored
/// (`Manifest::definition` without parts as written): every parameter but `ztp.config`, which
/// holds the file's contents, and `environment_variables`, whose `$VAR`s hold their values. The
/// file contents in it are not stored. It never held provisioners or `provision_concurrency`.
/// Each dropped field is logged.
///
/// # Errors
///
/// Fails when the labs cannot be listed; nothing is deleted then.
#[instrument(skip(db), fields(labs = %labs.display()))]
pub async fn import_lab_definitions(db: &Database, labs: &Path) -> Result<ImportSummary> {
    let mut summary = ImportSummary::default();
    let mut without_definition = HashSet::new();
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
                without_definition.insert(lab.lab_id);
            }
        }
    }
    for lab in db::list_labs(db).await? {
        let stored = !without_definition.contains(&lab.lab_id);
        summary.deleted += delete_saved_manifests(&labs.join(&lab.lab_id), &lab.lab_id, stored);
    }
    Ok(summary)
}

/// Store what survives as written of the manifest saved in `lab_dir` as `lab`'s definition.
async fn import_lab(db: &Database, lab: &DbLab, lab_dir: &Path) -> Result<()> {
    let manifest = read_saved_manifest(lab_dir)?;
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

/// The manifest saved in `lab_dir`: `manifest.toml`, or a legacy `manifest.json` when there is
/// none.
fn read_saved_manifest(lab_dir: &Path) -> Result<Manifest> {
    match fs::read_to_string(lab_dir.join(SAVED_MANIFEST_FILE)) {
        Ok(contents) => {
            toml::from_str(&contents).context("The saved manifest is not a valid manifest")
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let contents = fs::read_to_string(lab_dir.join(LEGACY_SAVED_MANIFEST_FILE))
                .context("Unable to read the saved manifest")?;
            serde_json::from_str(&contents)
                .context("The saved legacy manifest is not a valid manifest")
        }
        Err(error) => Err(error).context("Unable to read the saved manifest"),
    }
}

/// Delete the saved manifests in `lab_dir`, the directory of lab `lab_id`: `manifest.toml` and
/// a legacy `manifest.json`, and nothing else. `stored` says whether the lab has a stored
/// definition; a file deleted from a lab without one, its import having logged why, is logged at
/// warn. A file that is not there is skipped, so this can run on every start; one that cannot
/// be deleted is logged and left. Returns how many files were deleted.
#[instrument(skip(lab_dir), fields(lab_dir = %lab_dir.display()), level = "debug")]
fn delete_saved_manifests(lab_dir: &Path, lab_id: &str, stored: bool) -> usize {
    let mut deleted = 0;
    for file in [SAVED_MANIFEST_FILE, LEGACY_SAVED_MANIFEST_FILE] {
        let path = lab_dir.join(file);
        match fs::remove_file(&path) {
            Ok(()) if stored => {
                tracing::info!(%lab_id, path = %path.display(), "Deleted the saved manifest; the lab's definition is stored");
                deleted += 1;
            }
            Ok(()) => {
                tracing::warn!(%lab_id, path = %path.display(), "Deleted the saved manifest of a lab that has no stored definition");
                deleted += 1;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(%lab_id, path = %path.display(), %error, "Failed to delete the saved manifest");
            }
        }
    }
    deleted
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use db::{
        DefaultRequest, LabDetails, LabNetworks, SecretKey, apply_schema, create_lab, create_node,
        create_user, open_in_memory, read_lab_definition, upsert_node_image,
    };
    use shared::data::{NodeConfig, NodeModel};

    use crate::services::error::ServiceError;

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
        for (index, (name, model)) in (1..).zip(nodes) {
            let image = upsert_node_image(&db, NodeConfig::get_model(*model), DefaultRequest::Keep)
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
                skipped: 0,
                deleted: 1,
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
        assert!(!labs.path().join("lab-0001/manifest.toml").exists());

        // A second start finds nothing to import, and nothing to delete.
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
        assert_eq!(summary.deleted, 1);
        assert!(!labs.path().join("lab-0001/manifest.json").exists());
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
                skipped: 1,
                deleted: 0,
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
            // Unreadable, it is deleted all the same, with a warning.
            assert_eq!(summary.deleted, 1, "{contents}");
            assert!(!manifest.exists(), "{contents}");
        }
        // A file the server cannot open.
        fs::write(&manifest, MANIFEST_TOML).unwrap();
        fs::set_permissions(&manifest, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&manifest).is_err() {
            let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
            assert_eq!(summary.skipped, 1);
            assert_eq!(summary.deleted, 1);
            assert!(!manifest.exists());
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
        // The import logged why it stored none, so the file is deleted.
        assert_eq!(summary.deleted, 1);
        assert!(!labs.path().join("lab-0001/manifest.toml").exists());
        let stored = read_lab_definition(&db, lab.id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(stored, None);
    }

    #[tokio::test]
    async fn a_lab_with_a_stored_definition_has_its_saved_manifests_deleted() {
        let (db, lab) = lab().await;
        store(&db, &lab, &toml::to_string(&expected()).unwrap()).await;
        let labs = tempfile::tempdir().unwrap();
        let lab_dir = labs.path().join("lab-0001");
        fs::create_dir(&lab_dir).unwrap();
        fs::write(lab_dir.join("manifest.toml"), MANIFEST_TOML).unwrap();
        fs::write(lab_dir.join("manifest.json"), "{ not json").unwrap();

        let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                imported: 0,
                skipped: 0,
                deleted: 2,
            }
        );
        assert!(!lab_dir.join("manifest.toml").exists());
        assert!(!lab_dir.join("manifest.json").exists());
        let stored = read_lab_definition(&db, lab.id.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(stored, Some(expected()));

        // Idempotent: the next start finds nothing to delete, and does not fail.
        let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
        assert_eq!(summary, ImportSummary::default());
    }

    #[tokio::test]
    async fn only_the_saved_manifests_are_deleted() {
        let (db, _lab) = lab().await;
        let labs = tempfile::tempdir().unwrap();
        let lab_dir = labs.path().join("lab-0001");
        let kept = [
            "lab-info.toml",
            "manifest.toml.bak",
            "Manifest.toml",
            "notes.txt",
            "configs/manifest.toml",
            "manifest.toml.d/manifest.toml",
        ];
        for file in kept {
            let path = lab_dir.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, file).unwrap();
        }
        fs::write(lab_dir.join("manifest.toml"), MANIFEST_TOML).unwrap();
        // A directory with no lab record is not a lab's, and is left alone.
        let other = labs.path().join("lab-9999");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("manifest.toml"), MANIFEST_TOML).unwrap();
        fs::write(other.join("manifest.json"), "{}").unwrap();

        let summary = import_lab_definitions(&db, labs.path()).await.unwrap();
        assert_eq!(summary.deleted, 1);
        assert!(!lab_dir.join("manifest.toml").exists());
        for file in kept {
            assert_eq!(fs::read_to_string(lab_dir.join(file)).unwrap(), file);
        }
        assert!(other.join("manifest.toml").exists());
        assert!(other.join("manifest.json").exists());
    }

    #[tokio::test]
    async fn the_readers_get_the_stored_definition_or_none() {
        let (db, lab) = lab().await;
        let lab_id = lab.id.clone().unwrap();
        assert_eq!(stored_definition(&db, &lab_id, "lab-0001").await, None);
        store(&db, &lab, &toml::to_string(&expected()).unwrap()).await;
        assert_eq!(
            stored_definition(&db, &lab_id, "lab-0001").await,
            Some(expected())
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
            Some(&ServiceError::Conflict(
                "Node 'files01' cannot be redeployed from the web UI: it needs ztp.config, \
                 startup_scripts, which the server does not keep. Run `sherpa redeploy -n \
                 files01` from the lab folder"
                    .to_owned()
            ))
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
