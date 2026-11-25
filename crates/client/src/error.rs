use std::path::PathBuf;

use thiserror::Error;

use shared::data::ScannedImage;
use shared::konst::SHERPA_MANIFEST_FILE;

use crate::cmd::Route;

/// A client failure that a caller tells apart from the others.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("Failed to open the {route} of {node} through the Sherpa server")]
    ConsoleUnreachable { route: Route, node: String },

    #[error("Not logged in to the Sherpa server, or the login has expired: run 'sherpa login'.")]
    ConsoleLoginRefused,

    #[error("{message}")]
    ConsoleRefused { message: String },

    #[error(
        "The Sherpa server has no {route} route: it is older than this client. Upgrade the \
         server."
    )]
    NoConsoleRoute { route: Route },

    #[error("The Sherpa server refused the {route} of {node} (HTTP {status}).")]
    ConsoleRefusedWithStatus {
        route: Route,
        node: String,
        status: u16,
    },

    #[error("No token found. Please run: sherpa login")]
    NoToken,

    #[error("The login token is not a JWT; run: sherpa login")]
    TokenNotJwt,

    #[error("{} is a symlink; Sherpa keeps lab files only in a real folder", path.display())]
    SymlinkedFolder { path: PathBuf },

    #[error("{} is not a folder", path.display())]
    NotAFolder { path: PathBuf },

    #[error("{} is a symlink, so it was not deleted", path.display())]
    SymlinkNotDeleted { path: PathBuf },

    #[error("Failed to write {what}: {}", path.display())]
    SecretNotWritten { what: String, path: PathBuf },

    #[error("Failed to move the lab files into .sherpa")]
    LabFilesNotMoved,

    #[error("Failed to load the client config {path}")]
    ClientConfigNotLoaded { path: String },

    #[error(
        "{} is for lab {name}-{id}, but {SHERPA_MANIFEST_FILE} is for lab {manifest_name}; \
         destroy lab {name}-{id} or remove {} before using this folder for lab {manifest_name}",
        path.display(),
        path.display()
    )]
    LabsDiffer {
        path: PathBuf,
        name: String,
        id: String,
        manifest_name: String,
    },

    #[error("This folder has no {SHERPA_MANIFEST_FILE} or {}", lab_info.display())]
    NoLabFiles { lab_info: PathBuf },

    #[error(
        "This folder has no {SHERPA_MANIFEST_FILE}, and 'sherpa {command}' needs the lab's \
         source manifest. A folder made by 'sherpa download' has none: run 'sherpa {command}' \
         in the folder the lab was started from, or copy its {SHERPA_MANIFEST_FILE} here."
    )]
    NoSourceManifest { command: String },

    #[error(
        "{} does not parse; restore it with 'sherpa download' of the lab, or remove it if this \
         folder is for the lab named in {SHERPA_MANIFEST_FILE}",
        path.display()
    )]
    LabInfoUnparsable { path: PathBuf },

    #[error("Only the owner of lab '{lab_id}' can connect to its nodes")]
    NotLabOwner { lab_id: String },

    #[error("no link in the lab has the end {endpoint}")]
    NoLabLinkEnd { endpoint: String },

    #[error("no link in {SHERPA_MANIFEST_FILE} has the end {endpoint}")]
    NoManifestLinkEnd { endpoint: String },

    #[error(
        "The Sherpa server did not give the lab's link indexes: it is older than this client. \
         Upgrade the server, or run 'sherpa link impair' in the folder the lab was started \
         from, with its {SHERPA_MANIFEST_FILE}."
    )]
    NoLinkIndexes,

    #[error("Node '{node}' not found in {SHERPA_MANIFEST_FILE}")]
    NodeNotInManifest { node: String },

    #[error(
        "Lab connection files are missing. Start the lab with 'sherpa up', or restore existing \
         lab files with 'sherpa download {lab_id} --force'."
    )]
    LabConnectionFilesMissing { lab_id: String },

    #[error("Failed to read lab connection file: {}", path.display())]
    LabConnectionFileUnreadable { path: PathBuf },

    #[error("Node '{node}' not found in lab '{lab_id}'")]
    NodeNotInLab { node: String, lab_id: String },

    #[error("Saved SSH metadata belongs to a different lab")]
    ConnectionFilesOfAnotherLab,

    #[error("Server '{input}' must be a host, host:port, or a ws:// or wss:// URL")]
    UnsupportedServer { input: String },

    #[error("Server '{input}' names no host")]
    ServerWithoutHost { input: String },

    #[error(
        "Lab {lab_name}-{lab_id} was removed, but not all of its resources were cleaned up; see \
         the errors above"
    )]
    LabResourcesLeft { lab_name: String, lab_id: String },

    #[error(
        "Lab {lab_name}-{lab_id} was not fully destroyed and still exists on the server; fix the \
         problem, then run `{retry}` again"
    )]
    LabNotDestroyed {
        lab_name: String,
        lab_id: String,
        retry: String,
    },

    #[error("{} of {total} nodes were not {action}: {}", failed.len(), failed.join(", "))]
    NodesFailed {
        action: String,
        failed: Vec<String>,
        total: usize,
    },

    #[error(
        "{} of {total} images found were not recorded: {}",
        unrecorded.len(),
        images(unrecorded)
    )]
    ImagesNotRecorded {
        unrecorded: Vec<ScannedImage>,
        total: usize,
    },

    #[error("--model and --repo record container images: give them with --kind container")]
    ScanModelWithoutContainerKind,

    #[error(
        "Image reference '{reference}' must be image:tag, such as nats:2.11, or \
         registry/image:tag, such as ghcr.io/nokia/srlinux:1.2.3; digests (@sha256:...) are not \
         supported"
    )]
    InvalidImageReference { reference: String },

    #[error("Failed to read password")]
    PasswordNotRead,

    #[error("The password cannot be empty")]
    EmptyPassword,

    #[error("Passwords do not match")]
    PasswordsDiffer,

    #[error(
        "Lab files already exist in this directory ({}).\nUse --force to overwrite.",
        path.display()
    )]
    LabFilesExist { path: PathBuf },

    #[error("Tailscale auth-key environment variable '{name}' is not set or is not valid Unicode")]
    TailscaleKeyUnset { name: String },

    #[error("Failed to read ztp.config for node '{node}': {}", path.display())]
    ZtpConfigUnreadable { node: String, path: PathBuf },

    #[error("ztp.config for node '{node}' is empty: {}", path.display())]
    EmptyZtpConfig { node: String, path: PathBuf },

    #[error("startup_script for node '{node}' is empty: {}", path.display())]
    EmptyStartupScript { node: String, path: PathBuf },

    #[error("Failed to read text_file for node '{node}'")]
    TextFileUnreadable { node: String },

    #[error("Invalid environment variable '{entry}' for node '{node}': must be KEY=VALUE format")]
    InvalidEnvironmentEntry { entry: String, node: String },

    #[error("Environment variable '{variable}' referenced in node '{node}' is not set")]
    UnsetEnvironmentVariable { variable: String, node: String },

    #[error("Cannot read provisioner file {}", path.display())]
    ProvisionerFileUnreadable { path: PathBuf },

    #[error("line {line}: {variable} is not set in this environment")]
    UnsetEnvFileVariable { line: usize, variable: String },

    #[error("Env file {}", path.display())]
    EnvFile { path: PathBuf },

    #[error(
        "{node_name} at {address} is not reachable from this machine ({cause}). Use --ssh-jump \
         to go through the Sherpa server, or join the lab's tailnet and accept its routes."
    )]
    NoRouteToNode {
        node_name: String,
        address: String,
        cause: String,
    },

    #[error(
        "{} not found: run `sherpa up` or `sherpa download` in the lab folder first",
        path.display()
    )]
    NoLabSshConfig { path: PathBuf },

    #[error(
        "{failed} of {total} nodes were not provisioned; fix the problem, then run `sherpa \
         provision`"
    )]
    NodesNotProvisioned { failed: usize, total: usize },

    #[error("Node '{node}' is not in the manifest")]
    UnknownProvisionNode { node: String },

    #[error(
        "{host} is not in the lab SSH config; was the node added to the manifest after the lab \
         was created?"
    )]
    HostNotInSshConfig { host: String },
}

/// The model, kind and version of each image, separated by commas.
fn images(images: &[ScannedImage]) -> String {
    images
        .iter()
        .map(|image| format!("{} {} {}", image.model, image.kind, image.version))
        .collect::<Vec<_>>()
        .join(", ")
}
