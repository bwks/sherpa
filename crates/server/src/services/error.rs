//! Typed failures that services return inside `anyhow::Error`, and what a caller is told of
//! any other error.
//!
//! Transports find a `ServiceError` with [`ServiceError::find`] and map it to their own status
//! codes (REST status, RPC error code, HTML template), instead of matching on error text. An
//! error with no `ServiceError` in its chain is [`Unexpected`]: the caller is told only a
//! reference, and the server logs the reference with the error's chain.

use std::error;
use std::fmt;
use std::path::PathBuf;
use std::process::ExitStatus;

use db::{CpuPinTaken, UserChanged};
use nix::unistd::Pid;
use shared::data::{ModelKind, NodeKind, NodeModel};
use shared::error::{CpuPinConflict, SharedError};
use shared::konst::{ADMIN_USERNAME, SHERPA_USERNAME};
use thiserror::Error;
use uuid::Uuid;

use crate::api::request_parse::InvalidRequest;
use crate::auth::jwt;
use crate::daemon::pidfile::SHERPAD_LOG_FILE_PATH;
use crate::konst::SHERPA_ADMIN_PASSWORD_ENV;
use crate::validate::{DevicePasswordRule, MemoryShortage};

/// What a refusal of a node's version or image starts with.
pub const VERSION_CHECK_FAILED: &str = "Manifest validation failed: version/image validation";
/// Hex digits of an [`Unexpected`] reference.
const REFERENCE_LENGTH: usize = 8;

/// A service refused a request for a reason the caller can act on, or failed in a way a caller
/// tells apart from other failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServiceError {
    #[error("{0}")]
    NotFound(String),

    #[error("Access denied: {0}")]
    Denied(String),

    #[error("{0}")]
    Conflict(String),

    #[error("{0}")]
    Invalid(String),

    #[error("The device password does not meet the requirements: {rule}")]
    DevicePasswordRefused { rule: DevicePasswordRule },

    #[error("The device password must differ from your Sherpa login password")]
    DevicePasswordSameAsLogin,

    #[error(
        "Node '{node}' ({model_kind}) takes login names of at most {max_length} characters; \
         '{username}' has {length}. Start this lab from an account with a shorter name."
    )]
    LoginNameTooLong {
        node: String,
        model_kind: ModelKind,
        username: String,
        max_length: usize,
        length: usize,
    },

    #[error(
        "Node '{node}' ({model_kind}) takes login names that start with a letter or a digit; \
         '{username}' does not. Start this lab from an account whose name does."
    )]
    LoginNameFirstCharacter {
        node: String,
        model_kind: ModelKind,
        username: String,
    },

    #[error("{field} must not be empty")]
    LabOriginEmpty { field: &'static str },

    #[error("{field} must be at most {max_length} characters")]
    LabOriginTooLong {
        field: &'static str,
        max_length: usize,
    },

    #[error("{field} must not contain control characters")]
    LabOriginControlCharacters { field: &'static str },

    #[error(
        "Node '{node}' `{setting}` uses host CPU {cpu}, but the server has CPUs 0 to {}",
        host_cpus.saturating_sub(1)
    )]
    HostCpuMissing {
        node: String,
        setting: &'static str,
        cpu: u16,
        host_cpus: u32,
    },

    #[error(
        "Node '{node}' `numa.node` is NUMA node {numa_node}, but the server has \
         {host_numa_nodes} NUMA node(s), numbered from 0"
    )]
    NumaNodeMissing {
        node: String,
        numa_node: u8,
        host_numa_nodes: u32,
    },

    #[error(
        "Node '{node}' `cpu.count` is {cpu_count}, but the server has {host_cpus} CPUs; \
         a container's CPU quota cannot be more than the server's CPUs"
    )]
    ContainerCpusOverHost {
        node: String,
        cpu_count: u8,
        host_cpus: u32,
    },

    #[error("The server has no NUMA node {numa_node}")]
    NumaNodeNotOnServer { numa_node: u32 },

    #[error("`host_memory.overcommit_percent` must be at least 1, not {percent}")]
    OvercommitPercentTooLow { percent: u32 },

    #[error("{shortage}")]
    InsufficientMemory { shortage: MemoryShortage },

    #[error("ZTP validation failed for node: {node}: {refusal}")]
    NodeZtpUnsupported { node: String, refusal: SharedError },

    #[error(
        "Node '{node}' joins talos cluster '{cluster}' but its ZTP is disabled, so the machine \
         config Sherpa generates for it would not reach it"
    )]
    TalosMemberZtpDisabled { node: String, cluster: String },

    #[error("{refusal}")]
    ImageZtpUnsupported { refusal: SharedError },

    #[error("CPU count must be at least 1")]
    ImageCpuCountZero,

    #[error("Memory must be at least 64 MB")]
    ImageMemoryTooLow,

    #[error("Interface MTU must be between 576 and 9600")]
    ImageMtuOutOfRange,

    #[error(
        "Node '{node}' has provisioners, which only the sherpa CLI can run: start the lab with \
         `sherpa up`, or remove them"
    )]
    NodeHasProvisioners { node: String },

    #[error(
        "sherpad {command} must run as the '{user}' service account, not '{account}'. Nothing \
         was changed. Run it as: sudo -u {user} -H sherpad {command}, with the same options",
        user = SHERPA_USERNAME
    )]
    NotServiceAccount { command: String, account: String },

    #[error("Secret files were sent for node '{node}', which is not in the manifest")]
    SecretsForUnknownNode { node: String },

    #[error("Secret files were sent for node '{node}', but only containers take them")]
    SecretsForNonContainer { node: String },

    #[error("Node '{node}'")]
    NodeSecretFiles { node: String },

    #[error(
        "tailscale.image must be a nonempty container image reference without whitespace or NULs"
    )]
    TailscaleImageInvalid,

    #[error(
        "tailscale.socket_path must be an absolute Unix socket filename shorter than 108 bytes without NULs"
    )]
    TailscaleSocketPathInvalid,

    #[error("tailscale.{field} must be positive seconds")]
    TailscaleTimeoutNotPositive { field: &'static str },

    #[error("tailscale.stop_timeout_secs must fit a signed 32-bit Docker stop timeout")]
    TailscaleStopTimeoutTooLong,

    #[error(
        "tailscale.exec_timeout_secs ({exec_timeout_secs}) must be longer than tailscale.enrollment_timeout_secs ({enrollment_timeout_secs}), so the enrollment command ends before the server stops waiting for it"
    )]
    TailscaleExecTimeoutTooShort {
        exec_timeout_secs: u32,
        enrollment_timeout_secs: u32,
    },

    #[error(
        "Node '{node}' cannot be redeployed from the web UI: it needs {}, which the server does \
         not keep. Run `sherpa redeploy -n {node}` from the lab folder",
        missing.join(", ")
    )]
    NotWebRedeployable {
        node: String,
        missing: Vec<&'static str>,
    },

    #[error(
        "--model and --repo are given together: the repo's local tags are recorded as versions \
         of the model"
    )]
    ScanModelWithoutRepo,

    #[error("--model and --repo only scan containers: give them with --kind container")]
    ScanKindNotContainer,

    #[error("'{model}' is not a container model: --model and --repo record local Docker images")]
    ScanModelNotContainer { model: NodeModel },

    #[error(
        "'{repo}' is not a valid repo: use an image repository without a tag, such as mysql, \
         docker.io/library/mysql or ghcr.io/nokia/srlinux"
    )]
    ScanRepoInvalid { repo: String },

    #[error(
        "{prefix}: Node config not found for model: {model} kind: {kind} (node: {node})",
        prefix = VERSION_CHECK_FAILED,
        model = model_kind.model(),
        kind = model_kind.kind()
    )]
    NodeImageNotFound { node: String, model_kind: ModelKind },

    #[error(
        "{prefix}: Version validation failed for node: {node} (model: {model}): {}",
        version_refusal(*model_kind, version, available),
        prefix = VERSION_CHECK_FAILED,
        model = model_kind.model()
    )]
    VersionNotFound {
        node: String,
        model_kind: ModelKind,
        version: String,
        available: Vec<String>,
    },

    #[error(
        "Container node '{node}' (model: {model}) has no image repo configured: set `image` on \
         the node in the manifest, or pull the image with `sherpa server image pull --model \
         {model} --kind container --repo <image>:{version}`"
    )]
    ContainerRepoMissing {
        node: String,
        model: NodeModel,
        version: String,
    },

    #[error(
        "{prefix}: Disk image not found for node '{node}' (model: {model}, version: {version})\nExpected path: {path}\nHint: Ensure the image is installed in the correct directory",
        prefix = VERSION_CHECK_FAILED
    )]
    DiskImageNotFound {
        node: String,
        model: NodeModel,
        version: String,
        path: String,
    },

    #[error("The form sets `{field}`, which a {kind} image does not take")]
    ImageFormFieldNotTaken { field: &'static str, kind: NodeKind },

    #[error("The form has no `{field}`, which a {kind} image needs")]
    ImageFormFieldMissing { field: &'static str, kind: NodeKind },

    #[error(
        "The JWT secret {} is {length} bytes long, not {expected}. Restore it from a backup, or \
         delete it to generate a new one, which logs every user out",
        path.display(),
        expected = jwt::SECRET_LENGTH
    )]
    JwtSecretWrongLength { path: PathBuf, length: usize },

    #[error(
        "The database key {} is missing, and the database's secret values are sealed with it. \
         Restore the key file that was backed up with this database; a new key cannot open them",
        path.display()
    )]
    DbKeyMissing { path: PathBuf },

    #[error("The database key {} is damaged", path.display())]
    DbKeyDamaged { path: PathBuf },

    #[error(
        "Failed to start sherpad: it exited at once ({status}); see {log}",
        log = SHERPAD_LOG_FILE_PATH
    )]
    ExitedAtOnce { status: ExitStatus },

    #[error("Server is already running (PID: {pid}): {path} is locked")]
    PidFileLocked { pid: Pid, path: String },

    #[error(
        "The TLS {kind} {} does not exist. With self_generate_cert off, sherpad serves the \
         certificate and key the admin puts there, and never writes them",
        path.display()
    )]
    TlsFileMissing { kind: &'static str, path: PathBuf },

    #[error("No certificates found in file: {}", path.display())]
    NoCertificates { path: PathBuf },

    #[error(
        "sherpad is running (pid: {pid}) and holds the database. Nothing was changed. Stop it \
         first with: sudo systemctl stop sherpad, then start it again after the unlock"
    )]
    ServerHoldsDatabase { pid: Pid },

    #[error("User not found: {username}. Nothing was changed")]
    UnlockUserNotFound { username: String },

    #[error(
        "The built-in '{admin}' account does not exist. Run 'sudo -u sherpa -H sherpad init', or \
         set {env} to create it on startup",
        admin = ADMIN_USERNAME,
        env = SHERPA_ADMIN_PASSWORD_ENV
    )]
    AdminAccountMissing,

    #[error("Two secret files are named '{name}'")]
    SecretFileNameTwice { name: String },

    #[error("Secret file '{name}' is not valid base64")]
    SecretFileNotBase64 { name: String },

    #[error("{message}")]
    CpuPinsHeld {
        message: String,
        conflict: CpuPinConflict,
    },

    #[error("Failed to deserialize {what}")]
    RequestUnreadable {
        what: &'static str,
        #[source]
        refusal: InvalidRequest,
    },
}

/// The kind of refusal a transport answers a [`ServiceError`] with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCategory {
    NotFound,
    Denied,
    Conflict,
    Invalid,
}

/// Why `version` is not an image of `model_kind`: it has none, or the versions it has.
fn version_refusal(model_kind: ModelKind, version: &str, available: &[String]) -> String {
    let (model, kind) = (model_kind.model(), model_kind.kind());
    if available.is_empty() {
        format!("No configurations found in database for model: {model} kind: {kind}")
    } else {
        format!(
            "Version '{version}' not found in database for model: {model} kind: {kind}. \
             Available versions: {}",
            available.join(", ")
        )
    }
}

/// An error of no `ServiceError` kind, as a caller sees it: a reference to the server log line
/// that holds the error's chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unexpected {
    reference: String,
}

impl Unexpected {
    /// Log `error`, which `failure` describes, at error level with its chain (its alternate
    /// form, which for an `anyhow::Error` is the chain) and a new random reference, and return
    /// the reference for the caller.
    pub fn log(error: &dyn fmt::Display, failure: &str) -> Self {
        let unexpected = Self::new();
        tracing::error!(reference = %unexpected.reference, error = %format!("{error:#}"), "{failure}");
        unexpected
    }

    /// Log `error`, which `failure` describes, as [`Unexpected::log`] does but at warn level:
    /// for a problem that the server works around and logs once, not on each try.
    pub fn warn(error: &dyn fmt::Display, failure: &str) -> Self {
        let unexpected = Self::new();
        tracing::warn!(reference = %unexpected.reference, error = %format!("{error:#}"), "{failure}");
        unexpected
    }

    /// A new random reference.
    fn new() -> Self {
        let mut reference = Uuid::new_v4().simple().to_string();
        reference.truncate(REFERENCE_LENGTH);
        Self { reference }
    }
}

impl fmt::Display for Unexpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "internal error (ref {}); see the server log",
            self.reference
        )
    }
}

/// What a caller is told of `error`, which `failure` describes: the message of the
/// `ServiceError` in its chain, or else the reference of the [`Unexpected`] error it is logged
/// as.
pub fn caller_message(error: &anyhow::Error, failure: &str) -> String {
    match ServiceError::find(error) {
        Some(refusal) => refusal.message(),
        None => Unexpected::log(error, failure).to_string(),
    }
}

impl ServiceError {
    /// Find a `ServiceError` that refuses the request anywhere in an error chain.
    #[must_use]
    pub fn find(error: &anyhow::Error) -> Option<&ServiceError> {
        error
            .downcast_ref::<ServiceError>()
            .filter(|refusal| refusal.category().is_some())
    }

    /// What a caller is told of this refusal: its message, then the message of each error it
    /// carries as its source, separated by `: `.
    #[must_use]
    pub fn message(&self) -> String {
        let mut message = self.to_string();
        let mut source = error::Error::source(self);
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }
        message
    }

    /// The kind of refusal this is, or `None` for a failure the caller is not told of.
    #[must_use]
    pub fn category(&self) -> Option<ServiceCategory> {
        match self {
            Self::NotFound(_)
            | Self::NodeImageNotFound { .. }
            | Self::VersionNotFound { .. }
            | Self::DiskImageNotFound { .. } => Some(ServiceCategory::NotFound),
            Self::Denied(_) => Some(ServiceCategory::Denied),
            Self::Conflict(_)
            | Self::NotWebRedeployable { .. }
            | Self::ContainerRepoMissing { .. }
            | Self::InsufficientMemory { .. }
            | Self::CpuPinsHeld { .. } => Some(ServiceCategory::Conflict),
            Self::Invalid(_)
            | Self::DevicePasswordRefused { .. }
            | Self::DevicePasswordSameAsLogin
            | Self::LoginNameTooLong { .. }
            | Self::LoginNameFirstCharacter { .. }
            | Self::LabOriginEmpty { .. }
            | Self::LabOriginTooLong { .. }
            | Self::LabOriginControlCharacters { .. }
            | Self::HostCpuMissing { .. }
            | Self::NumaNodeMissing { .. }
            | Self::ContainerCpusOverHost { .. }
            | Self::NumaNodeNotOnServer { .. }
            | Self::NodeZtpUnsupported { .. }
            | Self::TalosMemberZtpDisabled { .. }
            | Self::ImageZtpUnsupported { .. }
            | Self::ImageCpuCountZero
            | Self::ImageMemoryTooLow
            | Self::ImageMtuOutOfRange
            | Self::NodeHasProvisioners { .. }
            | Self::SecretsForUnknownNode { .. }
            | Self::SecretsForNonContainer { .. }
            | Self::NodeSecretFiles { .. }
            | Self::SecretFileNameTwice { .. }
            | Self::SecretFileNotBase64 { .. }
            | Self::ScanModelWithoutRepo
            | Self::ScanKindNotContainer
            | Self::ScanModelNotContainer { .. }
            | Self::ScanRepoInvalid { .. }
            | Self::ImageFormFieldNotTaken { .. }
            | Self::ImageFormFieldMissing { .. }
            | Self::RequestUnreadable { .. } => Some(ServiceCategory::Invalid),
            Self::OvercommitPercentTooLow { .. }
            | Self::NotServiceAccount { .. }
            | Self::TailscaleImageInvalid
            | Self::TailscaleSocketPathInvalid
            | Self::TailscaleTimeoutNotPositive { .. }
            | Self::TailscaleStopTimeoutTooLong
            | Self::TailscaleExecTimeoutTooShort { .. }
            | Self::JwtSecretWrongLength { .. }
            | Self::DbKeyMissing { .. }
            | Self::DbKeyDamaged { .. }
            | Self::ExitedAtOnce { .. }
            | Self::PidFileLocked { .. }
            | Self::TlsFileMissing { .. }
            | Self::NoCertificates { .. }
            | Self::ServerHoldsDatabase { .. }
            | Self::UnlockUserNotFound { .. }
            | Self::AdminAccountMissing => None,
        }
    }

    /// The error of a failed `db::update_user`: a write refused because the user changed since
    /// it was read becomes a [`ServiceError::Conflict`] that tells the caller to try again.
    #[must_use]
    pub fn from_user_write(error: anyhow::Error) -> anyhow::Error {
        match error.downcast_ref::<UserChanged>() {
            Some(changed) => Self::Conflict(changed.to_string()).into(),
            None => error,
        }
    }

    /// The error of a failed `db::claim_cpu_pins`: a host CPU the database's trigger refused,
    /// as another node holds it, becomes the same [`ServiceError::CpuPinsHeld`] admission
    /// refuses with.
    #[must_use]
    pub fn from_cpu_pin_claim(error: anyhow::Error) -> anyhow::Error {
        match error.downcast_ref::<CpuPinTaken>() {
            Some(taken) => Self::CpuPinsHeld {
                message: taken.to_string(),
                conflict: CpuPinConflict {
                    host_cpus: vec![taken.host_cpu],
                    requested_role: taken.requested_role,
                    held_role: taken.held_role,
                },
            }
            .into(),
            None => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use shared::error::CpuPinRole;

    #[test]
    fn a_cpu_the_database_refused_to_pin_is_the_pin_refusal() {
        let error = ServiceError::from_cpu_pin_claim(
            CpuPinTaken {
                node_name: "n1".to_owned(),
                host_cpu: 3,
                requested_role: CpuPinRole::Thread,
                held_role: CpuPinRole::Vcpu,
            }
            .into(),
        );
        assert_eq!(
            ServiceError::find(&error),
            Some(&ServiceError::CpuPinsHeld {
                message: "Host CPU 3, which node 'n1' would pin to its QEMU threads, is pinned to \
                          the vCPUs of another node. Try again once that node is destroyed or \
                          redeployed without it"
                    .to_owned(),
                conflict: CpuPinConflict {
                    host_cpus: vec![3],
                    requested_role: CpuPinRole::Thread,
                    held_role: CpuPinRole::Vcpu,
                },
            })
        );
        let other = ServiceError::from_cpu_pin_claim(anyhow!("disk full"));
        assert!(ServiceError::find(&other).is_none());
    }
}
