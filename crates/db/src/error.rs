use thiserror::Error;
use turso::Value;

use shared::data::RecordId;

use crate::schema::ServerKeyLab;

/// A database operation's failure that a caller tells apart from the others.
#[derive(Debug, Error, PartialEq)]
pub enum DbError {
    #[error("lab_id must be exactly 8 characters long, got {length} characters")]
    LabIdLength { length: usize },

    #[error("lab_id must contain only alphanumeric characters and hyphens")]
    LabIdCharacters,

    #[error("Failed to create lab: '{name}'")]
    LabNotCreated { name: String },

    #[error("Failed to create node: name='{name}', index={index}, lab={lab:?}")]
    NodeNotCreated {
        name: String,
        index: u16,
        lab: RecordId,
    },

    #[error("Node not found with id: {id:?}")]
    NodeNotFound { id: RecordId },

    #[error("Node not found with name '{name}' in lab: {lab:?}")]
    NodeNotFoundInLab { name: String, lab: RecordId },

    #[error(
        "Failed to create link: index={index}, node_a={node_a:?}, node_b={node_b:?}, \
         int_a={int_a}, int_b={int_b}"
    )]
    LinkNotCreated {
        index: u16,
        node_a: RecordId,
        node_b: RecordId,
        int_a: String,
        int_b: String,
    },

    #[error("Link not found: {id:?}")]
    LinkNotFound { id: RecordId },

    #[error("User not found for deletion: {id:?}")]
    UserToDeleteNotFound { id: RecordId },

    #[error(
        "Refusing to store the contents of files as a lab definition: store the manifest as \
         written"
    )]
    DefinitionWithFileContents,

    #[error("Node '{node}' has no record in lab {lab_id}")]
    NodeWithoutRecord { node: String, lab_id: RecordId },

    #[error("Talos cluster {name} is not in the definition of lab {lab_id}")]
    TalosClusterNotInDefinition { name: String, lab_id: RecordId },

    #[error("Node {name}, used by a Talos cluster, has no management address")]
    TalosMemberWithoutAddress { name: String },

    #[error("Database field {field} is not a percentage between 0 and 100: {value}")]
    FieldNotPercentage { field: String, value: f64 },

    #[error("Database field {field} is not a number: {value:?}")]
    FieldNotNumber { field: String, value: Value },

    #[error("Failed to read row {row} of: {statement}")]
    RowUnreadable {
        row: String,
        statement: &'static str,
    },

    #[error(
        "The database has schema version {version}, newer than {supported} that this Sherpa \
         build supports"
    )]
    SchemaNewer { version: usize, supported: usize },

    #[error(
        "{} lab(s) still log in to their nodes with the server's shared SSH key, which this \
         release removes; their nodes would lose SSH access:\n{}\n\
         Destroy them with the Sherpa release this server ran before ('sherpa destroy' in the \
         lab's folder, or the web UI), then start this release again. The database is unchanged.",
        labs.len(),
        listed(labs)
    )]
    ServerKeyLabs { labs: Vec<ServerKeyLab> },

    #[error("The database key is not loaded, so secret values cannot be sealed or opened")]
    SecretKeyNotLoaded,

    #[error(
        "The database key does not match the key this database's secret values are sealed \
         with. Restore the key file that was backed up with this database"
    )]
    SecretKeyMismatch,

    #[error(
        "Failed to open {context}: it was sealed with a different database key, or has been \
         changed"
    )]
    SealedWithAnotherKey { context: String },

    #[error("{context} is not sealed")]
    NotSealed { context: String },
}

/// The labs on the server's key, one indented line each.
fn listed(labs: &[ServerKeyLab]) -> String {
    labs.iter()
        .map(|lab| format!("  {} ({}, owned by {})", lab.lab_id, lab.name, lab.owner))
        .collect::<Vec<_>>()
        .join("\n")
}
