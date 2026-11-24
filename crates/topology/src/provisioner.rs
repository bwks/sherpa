//! Provisioners, run by the `sherpa` CLI for a node.
//!
//! Provisioners name client files that can hold secrets, so `Node::provisioners` is never
//! serialized with the manifest. The client sends their definitions, paths and settings only,
//! beside it in a [`ManifestAsWritten`](crate::ManifestAsWritten), and the server stores those
//! with the lab. No provisioner's
//! file contents are stored: the ssh type's files never reach the server, and the container
//! type's are sent outside the manifest and kept in RAM on the server only.
use serde_derive::{Deserialize, Serialize};

use shared::konst::{CONTAINER_SECRET_DEFAULT_MODE, PROVISIONER_FILE_DEFAULT_MODE};

/// One provisioner on a node, selected by its `type`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Provisioner {
    Ssh(SshProvisioner),
    Container(ContainerProvisioner),
}

/// Secret files for a container, mounted read-only at `/run/secrets/<name>` before it starts.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContainerProvisioner {
    #[serde(default)]
    pub files: Vec<ContainerSecretFile>,
}

/// A client file mounted in a container as a secret.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContainerSecretFile {
    pub src: String,
    pub name: String,
    #[serde(default = "default_secret_mode")]
    pub mode: u32,
}

/// Copies files over SFTP and runs scripts in the CLI's own SSH connection, in this order:
/// `files`, `env_files`, `scripts`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SshProvisioner {
    #[serde(default)]
    pub files: Vec<ProvisionerFile>,
    #[serde(default)]
    pub env_files: Vec<String>,
    #[serde(default)]
    pub scripts: Vec<String>,
}

/// A client file copied to a node.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvisionerFile {
    pub src: String,
    pub dst: String,
    #[serde(default = "default_file_mode")]
    pub mode: u32,
}

/// Serde default for `ProvisionerFile::mode`. `#[serde(default = "...")]` takes a function path,
/// not a constant, so this returns `PROVISIONER_FILE_DEFAULT_MODE`.
fn default_file_mode() -> u32 {
    PROVISIONER_FILE_DEFAULT_MODE
}

/// Serde default for `ContainerSecretFile::mode`. `#[serde(default = "...")]` takes a function
/// path, not a constant, so this returns `CONTAINER_SECRET_DEFAULT_MODE`.
fn default_secret_mode() -> u32 {
    CONTAINER_SECRET_DEFAULT_MODE
}
