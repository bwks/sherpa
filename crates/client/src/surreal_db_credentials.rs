use anyhow::{Context, Result};
use serde_derive::Serialize;

use shared::data::SurrealDbInfo;
use shared::util::{Emoji, LabFolder};

use crate::lab_files::LabFile;

/// A `SurrealDB` node's credentials file, `surrealdb-<node>.toml`.
#[derive(Debug, Serialize)]
struct SurrealDbCredentialsFile<'a> {
    address: &'a str,
    username: &'a str,
    password: &'a str,
}

/// The credentials file of one `SurrealDB` node.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SurrealDbFile {
    pub name: String,
    pub address: String,
    pub file: LabFile,
}

/// Each `SurrealDB` node's address and root user as its file in `folder`,
/// `.sherpa/surrealdb/surrealdb-<node>.toml`, a secret. Writes nothing.
pub(crate) fn surreal_db_credential_files(
    folder: &LabFolder,
    surreal_dbs: &[SurrealDbInfo],
) -> Result<Vec<SurrealDbFile>> {
    let mut files = Vec::with_capacity(surreal_dbs.len());
    for surreal_db in surreal_dbs {
        let content = toml::to_string(&SurrealDbCredentialsFile {
            address: &surreal_db.address,
            username: &surreal_db.admin.username,
            password: &surreal_db.admin.password,
        })
        .context("Failed to serialize SurrealDB credentials")?;
        files.push(SurrealDbFile {
            name: surreal_db.name.clone(),
            address: surreal_db.address.clone(),
            file: LabFile {
                path: folder.surreal_db(&surreal_db.name),
                content,
                secret: Some("SurrealDB credentials"),
            },
        });
    }
    Ok(files)
}

/// Print where each `SurrealDB` node's credentials were written, and its address.
pub(crate) fn print_surreal_db_files(files: &[SurrealDbFile]) {
    for surreal_db in files {
        println!(
            "{} SurrealDB {} credentials: {}",
            Emoji::Success,
            surreal_db.name,
            surreal_db.file.path.display()
        );
        println!("   Endpoint: {}", surreal_db.address);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::lab_files::write_lab_file;

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
        let folder = LabFolder::new(dir.path());
        let files = surreal_db_credential_files(&folder, &surreal_dbs).unwrap();
        let [surreal_db] = files.as_slice() else {
            panic!("one file for one SurrealDB node: {files:?}");
        };
        write_lab_file(&folder, &surreal_db.file).unwrap();
        let path = dir.path().join(".sherpa/surrealdb/surrealdb-db01.toml");
        assert_eq!(
            (
                surreal_db.name.as_str(),
                surreal_db.address.as_str(),
                &surreal_db.file.path
            ),
            ("db01", "http://172.31.1.24:8000", &path)
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
