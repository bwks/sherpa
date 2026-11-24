//! `SurrealDB` nodes' root users. Each `SurrealDB` node starts with its own root user, `root`
//! with a generated password, made when the node is created and kept across `redeploy`. It is
//! saved in the lab directory, readable only by the server, and handed to the lab owner by `up`
//! and `download`.

use std::ffi::OsStr;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use shared::data::{AdminCredential, SurrealDbInfo};
use shared::konst::SHERPA_LABS_PATH;
use shared::util::write_file_with_permissions;

use crate::services::admin_credentials::random_password;

/// Directory in the lab directory holding each `SurrealDB` node's root user.
pub(crate) const SURREAL_DB_DIR: &str = "surrealdb";
/// The root user a `SurrealDB` node starts with, `SurrealDB`'s own default name.
const SURREAL_DB_ROOT_USERNAME: &str = "root";
/// The port `SurrealDB` serves its HTTP and WebSocket API on.
const SURREAL_DB_PORT: u16 = 8000;

/// A `SurrealDB` node's address and root user, saved as `<lab>/surrealdb/<node>.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SurrealDbCredentials {
    address: String,
    admin: AdminCredential,
}

/// The lab directory where Sherpa keeps its `SurrealDB` nodes' root users.
pub fn surreal_db_dir(lab_id: &str) -> PathBuf {
    Path::new(SHERPA_LABS_PATH)
        .join(lab_id)
        .join(SURREAL_DB_DIR)
}

/// The base address of a `SurrealDB` node.
pub fn surreal_db_address(node_address: Ipv4Addr) -> String {
    format!("http://{node_address}:{SURREAL_DB_PORT}")
}

fn credentials_path(dir: &Path, node_name: &str) -> PathBuf {
    dir.join(format!("{node_name}.toml"))
}

fn load_credentials(dir: &Path, node_name: &str) -> Result<Option<SurrealDbCredentials>> {
    let path = credentials_path(dir, node_name);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let credentials =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(Some(credentials))
}

/// The root user a `SurrealDB` node is created with, saved in `dir` before the node starts. A
/// node keeps what was saved for it, across `redeploy`: its database is in memory, so a
/// redeployed node starts empty with the same root user, and the owner's downloaded file stays
/// right. A node with none saved gets a new one.
#[instrument(skip_all, fields(dir = %dir.display(), %node_name), level = "debug")]
pub(crate) fn ensure_surreal_db_credentials(
    dir: &Path,
    node_name: &str,
    address: &str,
) -> Result<AdminCredential> {
    if let Some(saved) = load_credentials(dir, node_name)? {
        return Ok(saved.admin);
    }
    let credentials = SurrealDbCredentials {
        address: address.to_owned(),
        admin: AdminCredential {
            username: SURREAL_DB_ROOT_USERNAME.to_owned(),
            password: random_password(),
        },
    };
    let content =
        toml::to_string(&credentials).context("Failed to serialize SurrealDB credentials")?;
    write_file_with_permissions(&credentials_path(dir, node_name), content.as_bytes(), 0o600)
        .with_context(|| format!("Failed to save the root user of SurrealDB node '{node_name}'"))?;
    Ok(credentials.admin)
}

/// The root user of every `SurrealDB` node in a lab's `SurrealDB` directory (see
/// [`surreal_db_dir`]), for `up` and `download`.
#[instrument(level = "debug", fields(dir = %dir.display()))]
pub fn read_surreal_db_credentials(dir: &Path) -> Result<Vec<SurrealDbInfo>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut surreal_dbs = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to list {}", dir.display()))? {
        let path = entry?.path();
        if path.extension() != Some(OsStr::new("toml")) {
            continue;
        }
        let Some(node_name) = path.file_stem().and_then(OsStr::to_str) else {
            continue;
        };
        if let Some(credentials) = load_credentials(dir, node_name)? {
            surreal_dbs.push(SurrealDbInfo {
                name: node_name.to_owned(),
                address: credentials.address,
                admin: credentials.admin,
            });
        }
    }
    surreal_dbs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(surreal_dbs)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    const ADDRESS: &str = "http://172.31.1.24:8000";

    #[test]
    fn lab_credentials_010_a_new_surreal_db_node_gets_root_saved_for_sherpad() {
        let dir = TempDir::new().unwrap();
        let admin = ensure_surreal_db_credentials(dir.path(), "db01", ADDRESS).unwrap();
        assert_eq!(admin.username, "root");
        assert_eq!(admin.password.len(), 20);

        let path = credentials_path(dir.path(), "db01");
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            load_credentials(dir.path(), "db01").unwrap(),
            Some(SurrealDbCredentials {
                address: ADDRESS.to_owned(),
                admin,
            })
        );
    }

    #[test]
    fn lab_credentials_017_an_administrator_s_password_is_not_in_its_debug_output() {
        let dir = TempDir::new().unwrap();
        let admin = ensure_surreal_db_credentials(dir.path(), "db01", ADDRESS).unwrap();
        let debug = format!("{admin:?}");
        assert!(!debug.contains(&admin.password), "{debug}");
        assert!(debug.contains("root"), "{debug}");
    }
}
