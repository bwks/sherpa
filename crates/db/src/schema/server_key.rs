//! Labs that still log in to their nodes with the server's shared SSH key.
//!
//! Labs created before lab keys recorded `ssh_key = 'server'` on the lab. The release that
//! drops `lab.ssh_key` also removes the server's key, so their nodes would lose SSH access. The
//! schema is not migrated past that point while any such lab exists; the refusal names them,
//! so they can be destroyed first with the release the server ran before.
//!
//! These statements read a column the current schema no longer has, so they are not query
//! files: `build.rs` checks those against the current schema.

use anyhow::{Result, bail};
use tracing::instrument;
use turso::{Connection, Row, Value};

use crate::sql::{Query, query_all, query_count};

/// Whether the `lab` table still has its `ssh_key` column.
const HAS_SSH_KEY_COLUMN: Query<0> =
    Query::new("SELECT count(*) FROM pragma_table_info('lab') WHERE name = 'ssh_key'");

/// The labs on the server's key, with their owners.
const SERVER_KEY_LABS: Query<0> = Query::new(
    "SELECT lab.lab_id, lab.name, user.username FROM lab \
     JOIN user ON user.id = lab.user \
     WHERE lab.ssh_key = 'server' ORDER BY lab.lab_id",
);

/// A lab that logs in to its nodes with the server's shared key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerKeyLab {
    pub lab_id: String,
    pub name: String,
    pub owner: String,
}

fn server_key_lab_from_row(row: &Row) -> Result<ServerKeyLab> {
    let text = |index: usize| -> Result<String> {
        match row.get_value(index)? {
            Value::Text(text) => Ok(text),
            other => bail!("Lab column {index} is not text: {other:?}"),
        }
    };
    Ok(ServerKeyLab {
        lab_id: text(0)?,
        name: text(1)?,
        owner: text(2)?,
    })
}

/// The labs on the server's key. None once `lab.ssh_key` has been dropped.
#[instrument(skip(conn), level = "debug")]
pub(crate) async fn server_key_labs(conn: &Connection) -> Result<Vec<ServerKeyLab>> {
    if query_count(conn, HAS_SSH_KEY_COLUMN.bind([])).await? == 0 {
        return Ok(Vec::new());
    }
    query_all(conn, SERVER_KEY_LABS.bind([]), server_key_lab_from_row).await
}

/// Refuse to go on while any lab uses the server's key, naming each one.
///
/// # Errors
///
/// When `labs` is not empty.
pub(crate) fn refuse_server_key_labs(labs: &[ServerKeyLab]) -> Result<()> {
    if labs.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = labs
        .iter()
        .map(|lab| format!("  {} ({}, owned by {})", lab.lab_id, lab.name, lab.owner))
        .collect();
    bail!(
        "{count} lab(s) still log in to their nodes with the server's shared SSH key, which this \
         release removes; their nodes would lose SSH access:\n{names}\n\
         Destroy them with the Sherpa release this server ran before ('sherpa destroy' in the \
         lab's folder, or the web UI), then start this release again. The database is unchanged.",
        count = labs.len(),
        names = names.join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab(lab_id: &str, name: &str, owner: &str) -> ServerKeyLab {
        ServerKeyLab {
            lab_id: lab_id.to_owned(),
            name: name.to_owned(),
            owner: owner.to_owned(),
        }
    }

    #[test]
    fn no_lab_on_the_server_key_is_no_refusal() {
        assert!(refuse_server_key_labs(&[]).is_ok());
    }

    #[test]
    fn labs_on_the_server_key_are_refused_by_id_name_and_owner() {
        let labs = [
            lab("abcd1234", "core", "alice"),
            lab("efgh5678", "edge", "bob"),
        ];

        let error = refuse_server_key_labs(&labs).unwrap_err().to_string();

        assert!(error.starts_with("2 lab(s) still log in"), "{error}");
        assert!(
            error.contains("  abcd1234 (core, owned by alice)"),
            "{error}"
        );
        assert!(error.contains("  efgh5678 (edge, owned by bob)"), "{error}");
        assert!(error.contains("Destroy them"), "{error}");
        assert!(error.contains("The database is unchanged"), "{error}");
    }
}
