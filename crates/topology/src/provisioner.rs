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
    /// Copy files and run scripts over SSH, as the node's login user.
    Ssh(SshProvisioner),
    /// Mount secret files in a container at `/run/secrets`.
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
    /// Path on the client: `~` is expanded and relative paths start at the manifest.
    pub src: String,
    /// File name in `/run/secrets`.
    pub name: String,
    /// File mode as octal digits. Defaults to `444`, as the container may not run as root.
    #[serde(default = "default_secret_mode")]
    pub mode: u32,
}

/// Copies files with `scp` and runs scripts over SSH, in this order: `files`, `env_files`,
/// `scripts`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SshProvisioner {
    /// Client files copied to the node.
    #[serde(default)]
    pub files: Vec<ProvisionerFile>,
    /// Client env files (`KEY=value` lines) copied to `~/.env.d/`, which login shells load.
    #[serde(default)]
    pub env_files: Vec<String>,
    /// Client scripts run on the node in a login shell, through their shebang.
    #[serde(default)]
    pub scripts: Vec<String>,
}

/// A client file copied to a node.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvisionerFile {
    /// Path on the client: `~` is expanded and relative paths start at the manifest.
    pub src: String,
    /// Path on the node: absolute, or `~/` relative to the login user's home.
    pub dst: String,
    /// File mode as octal digits, for example `600`.
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
