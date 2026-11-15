//! The lab's definition in the database: the manifest parts the client sends beside the
//! manifest, and the one-time import of labs stored before definitions were.
//!
//! `up` stores a lab's definition with its records, and `redeploy` the node it rebuilds (see
//! `db::definition`). A lab stored before then has none; at startup sherpad imports it from the
//! lab's saved manifest, once.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tracing::instrument;

use db::{Database, StoredProvisioning};
use shared::data::DbLab;
use shared::util::load_saved_manifest;
use topology::validate::validate_provisioners;
use topology::{Manifest, Provisioning};

/// What the startup import did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Labs whose definition was imported from their saved manifest.
    pub imported: usize,
    /// Labs left without a definition: their saved manifest is missing, unreadable or does not
    /// fit their records. Each is logged.
    pub skipped: usize,
}

/// Add the provisioning the client sent beside `manifest`, checked as the client checks it.
/// Returns whether the stored provisioning is replaced: a request without it keeps it.
///
/// # Errors
///
/// Fails when `provisioning` is not a provisioning, names a node the manifest does not have, or
/// fails the provisioner checks.
pub fn add_provisioning(
    manifest: &mut Manifest,
    provisioning: Option<Value>,
) -> Result<StoredProvisioning> {
    let Some(provisioning) = provisioning else {
        return Ok(StoredProvisioning::Keep);
    };
    let provisioning: Provisioning =
        serde_json::from_value(provisioning).context("Failed to deserialize the provisioning")?;
    manifest.set_provisioning(provisioning)?;
    validate_provisioners(manifest)?;
    Ok(StoredProvisioning::Replace)
}

/// Store a definition for every lab that has none, from the manifest saved in its directory
/// under `labs`: `manifest.toml`, or a legacy `manifest.json`. A lab whose manifest is
/// missing, unreadable, or does not fit its records is logged and left without one; nothing is
/// made up for it. The saved files are left in place.
///
/// The saved manifest never held provisioners or `provision_concurrency`, so an imported lab
/// has none stored.
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

/// Store the manifest saved in `lab_dir` as `lab`'s definition.
async fn import_lab(db: &Database, lab: &DbLab, lab_dir: &Path) -> Result<()> {
    let saved = load_saved_manifest(lab_dir)?;
    let manifest: Manifest =
        serde_json::from_value(saved).context("The saved manifest is not a valid manifest")?;
    db::store_lab_definition(db, &db::get_lab_id(lab)?, &manifest)
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
environment_variables = ["MODE=lab"]

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

    fn expected() -> Manifest {
        toml::from_str(MANIFEST_TOML).unwrap()
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
            serde_json::to_string(&expected()).unwrap(),
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
    fn sent_provisioning_is_added_and_checked() {
        let mut manifest = expected();
        let kept = add_provisioning(&mut manifest, None).unwrap();
        assert_eq!(kept, StoredProvisioning::Keep);

        let sent = serde_json::json!({
            "concurrency": 2,
            "nodes": { "dev01": [{ "type": "ssh", "scripts": ["./setup.sh"] }] },
        });
        let replaced = add_provisioning(&mut manifest, Some(sent)).unwrap();
        assert_eq!(replaced, StoredProvisioning::Replace);
        assert_eq!(manifest.provision_concurrency, Some(2));
        assert_eq!(manifest.nodes[0].provisioners.as_ref().unwrap().len(), 1);

        let zero = serde_json::json!({ "concurrency": 0 });
        assert!(add_provisioning(&mut expected(), Some(zero)).is_err());
        let ghost = serde_json::json!({ "nodes": { "ghost": [] } });
        assert!(add_provisioning(&mut expected(), Some(ghost)).is_err());
    }
}
