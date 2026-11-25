use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};
use tokio::fs;
use tracing::instrument;

use db::Database;
use shared::data::DownloadLabResponse;

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_owner;
use crate::konst::{LAB_CA_CERT_FILE, LAB_CERTS_DIR};
use crate::services::keycloak;
use crate::services::keycloak::lab::KEYCLOAK_DIR;
use crate::services::lab_info;
use crate::services::ssh_config::{self, LabSshFiles};
use crate::services::surreal_db;
use crate::services::surreal_db::SURREAL_DB_DIR;
use crate::services::talos;
use crate::services::vault;
use crate::services::vault::lab::VAULT_DIR;

/// Download lab files (lab-info, SSH config, `known_hosts`) for CLI use.
///
/// The lab info, SSH config and `known_hosts` are built from the database; the server keeps
/// no copy of them. Only the lab's owner may download them, admins included: the files are for
/// connecting to the lab's nodes, and the cluster, Vault, Keycloak and `SurrealDB` credentials and
/// the lab CA are the owner's. Those are read from the lab's folder in `labs_root`, the server's
/// labs directory.
#[instrument(skip(ctx, db), fields(username = %ctx.username, %lab_id, labs_root = %labs_root.display()))]
pub async fn download_lab_files(
    ctx: &AuthContext,
    lab_id: &str,
    db: &Database,
    labs_root: &Path,
) -> Result<DownloadLabResponse> {
    let db_lab = require_lab_owner(ctx, db, lab_id).await?;
    let lab_record_id = db::get_lab_id(&db_lab).context("Failed to get lab record ID")?;

    let lab_dir = labs_root.join(lab_id);
    let lab_info = lab_info::load_lab_info(db, &db_lab).await?;
    let LabSshFiles {
        ssh_config,
        known_hosts,
    } = ssh_config::lab_ssh_files(db, &lab_record_id, &lab_info).await?;

    let clusters = talos::read_cluster_credentials(db, labs_root, &lab_record_id, lab_id).await?;

    // Labs created before the lab CA have no CA certificate.
    let lab_ca_path = lab_dir.join(LAB_CERTS_DIR).join(LAB_CA_CERT_FILE);
    let lab_ca = match fs::read_to_string(&lab_ca_path).await {
        Ok(lab_ca) => lab_ca,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            tracing::debug!(path = %lab_ca_path.display(), "Lab has no CA certificate");
            String::new()
        }
        Err(error) => {
            return Err(error).context(format!("Failed to read {}", lab_ca_path.display()));
        }
    };
    let vaults = vault::read_vault_credentials(&lab_dir.join(VAULT_DIR))?;
    let keycloaks = keycloak::read_keycloak_credentials(&lab_dir.join(KEYCLOAK_DIR))?;
    let surreal_dbs = surreal_db::read_surreal_db_credentials(&lab_dir.join(SURREAL_DB_DIR))?;

    Ok(DownloadLabResponse {
        lab_info,
        ssh_config,
        known_hosts,
        clusters,
        lab_ca,
        vaults,
        keycloaks,
        surreal_dbs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::data::AdminCredential;

    use crate::services::error::ServiceError;
    use crate::services::keycloak::lab::ensure_keycloak_credentials;
    use crate::services::node_lifecycle::tests::lab_of;
    use crate::services::surreal_db::ensure_surreal_db_credentials;

    /// A lab owned by `alice`, with a Keycloak and a `SurrealDB` node's admins saved in the
    /// labs directory `labs_root`, and those admins.
    async fn lab_with_admins(
        lab_id: &str,
        labs_root: &Path,
    ) -> (Database, AdminCredential, AdminCredential) {
        let (db, _, _) = lab_of(lab_id, 1).await;
        let lab_dir = labs_root.join(lab_id);
        let keycloak = ensure_keycloak_credentials(
            &lab_dir.join(KEYCLOAK_DIR),
            "kc01",
            "https://172.31.1.23:8443",
            "alice",
        )
        .unwrap();
        let surreal_db = ensure_surreal_db_credentials(
            &lab_dir.join(SURREAL_DB_DIR),
            "db01",
            "http://172.31.1.24:8000",
        )
        .unwrap();
        (db, keycloak, surreal_db)
    }

    #[tokio::test]
    async fn lab_credentials_020_a_download_gives_each_application_node_s_admin() {
        let labs_root = tempfile::tempdir().unwrap();
        let (db, keycloak, surreal_db) = lab_with_admins("dl-own01", labs_root.path()).await;
        let alice = AuthContext::new("alice".to_owned(), false, "session".to_owned());

        let files = download_lab_files(&alice, "dl-own01", &db, labs_root.path())
            .await
            .unwrap();

        assert_eq!(files.keycloaks.len(), 1);
        assert_eq!(files.keycloaks[0].admin, Some(keycloak));
        assert_eq!(files.surreal_dbs.len(), 1);
        assert_eq!(files.surreal_dbs[0].name, "db01");
        assert_eq!(files.surreal_dbs[0].admin, surreal_db);
    }

    #[tokio::test]
    async fn an_admin_cannot_download_another_user_s_application_node_admins() {
        let labs_root = tempfile::tempdir().unwrap();
        let (db, _, _) = lab_with_admins("dl-adm01", labs_root.path()).await;
        let admin = AuthContext::new("admin".to_owned(), true, "session".to_owned());

        let error = download_lab_files(&admin, "dl-adm01", &db, labs_root.path())
            .await
            .unwrap_err();

        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::Denied(_))
            ),
            "{error:#}"
        );
    }
}
