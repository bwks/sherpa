use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_derive::Serialize;

use shared::data::SurrealDbInfo;
use shared::util::{Emoji, LabFolder};

use crate::lab_files::ensure_lab_subdir;
use crate::secret_file::write_secret_file;

/// A `SurrealDB` node's credentials file, `surrealdb-<node>.toml`.
#[derive(Debug, Serialize)]
struct SurrealDbCredentialsFile<'a> {
    address: &'a str,
    username: &'a str,
    password: &'a str,
}

/// The credentials file written for one `SurrealDB` node.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SurrealDbFile {
    pub name: String,
    pub address: String,
    pub path: PathBuf,
}

/// Write each `SurrealDB` node's address and root user into `folder` as
/// `.sherpa/surrealdb/surrealdb-<node>.toml`, readable only by the current user.
#[tracing::instrument(level = "debug", skip_all, fields(folder = %folder.root().display(), surreal_dbs = surreal_dbs.len()))]
pub(crate) fn write_surreal_db_credentials(
    folder: &LabFolder,
    surreal_dbs: &[SurrealDbInfo],
) -> Result<Vec<SurrealDbFile>> {
    if !surreal_dbs.is_empty() {
        ensure_lab_subdir(folder, &folder.surreal_db_dir())?;
    }
    let mut written = Vec::with_capacity(surreal_dbs.len());
    for surreal_db in surreal_dbs {
        let path = folder.surreal_db(&surreal_db.name);
        let content = toml::to_string(&SurrealDbCredentialsFile {
            address: &surreal_db.address,
            username: &surreal_db.admin.username,
            password: &surreal_db.admin.password,
        })
        .context("Failed to serialize SurrealDB credentials")?;
        write_secret_file(&path, &content, "SurrealDB credentials")?;
        written.push(SurrealDbFile {
            name: surreal_db.name.clone(),
            address: surreal_db.address.clone(),
            path,
        });
    }
    Ok(written)
}

/// Print where each `SurrealDB` node's credentials were written, and its address.
pub(crate) fn print_surreal_db_files(files: &[SurrealDbFile]) {
    for surreal_db in files {
        println!(
            "{} SurrealDB {} credentials: {}",
            Emoji::Success,
            surreal_db.name,
            surreal_db.path.display()
        );
        println!("   Endpoint: {}", surreal_db.address);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use shared::data::AdminCredential;

    #[test]
    fn lab_credentials_008_a_surreal_db_file_has_its_root_user_and_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let surreal_dbs = [SurrealDbInfo {
            name: "db01".to_owned(),
            address: "http://172.31.1.24:8000".to_owned(),
            admin: AdminCredential {
                username: "root".to_owned(),
                password: "db-root-pw".to_owned(),
            },
        }];
        let written =
            write_surreal_db_credentials(&LabFolder::new(dir.path()), &surreal_dbs).unwrap();
        let path = dir.path().join(".sherpa/surrealdb/surrealdb-db01.toml");
        assert_eq!(
            written,
            [SurrealDbFile {
                name: "db01".to_owned(),
                address: "http://172.31.1.24:8000".to_owned(),
                path: path.clone(),
            }]
        );
        let content: toml::Value =
            toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content["address"].as_str(), Some("http://172.31.1.24:8000"));
        assert_eq!(content["username"].as_str(), Some("root"));
        assert_eq!(content["password"].as_str(), Some("db-root-pw"));
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
