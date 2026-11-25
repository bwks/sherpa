use ssh_key::Algorithm;
use thiserror::Error;

use crate::data::{NodeKind, NodeModel, Ztp, model_kinds_list};
use crate::konst::{USERNAME_MAX_LENGTH, USERNAME_MIN_LENGTH};
use crate::validators::{MAX_IMPAIRMENT_DELAY_MS, PasswordRequirement};

/// The SSH key types the validator accepts, for its error message.
const SUPPORTED_SSH_KEY_ALGORITHMS: &str = "ssh-ed25519, ssh-rsa, ecdsa-sha2-nistp256, \
     ecdsa-sha2-nistp384, ecdsa-sha2-nistp521, sk-ssh-ed25519@openssh.com, \
     sk-ecdsa-sha2-nistp256@openssh.com";

/// A shared check's failure that a caller tells apart from the others.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum SharedError {
    #[error("Unknown node model: {name}")]
    UnknownNodeModel { name: String },

    #[error(
        "Impairment {field} of {value} ms is longer than the {max} ms netem can hold",
        max = MAX_IMPAIRMENT_DELAY_MS
    )]
    ImpairmentDelayTooLong { field: &'static str, value: u32 },

    #[error("Impairment {field} must be between 0 and 100, not {value}")]
    ImpairmentPercentOutOfRange { field: &'static str, value: f64 },

    #[error(
        "Model '{model}' has no default kind; set its kind to one of: {}",
        model_kinds_list(*model)
    )]
    NoDefaultKind { model: NodeModel },

    #[error(
        "Model '{model}' is not a {kind}; it comes only as: {}",
        model_kinds_list(*model)
    )]
    KindNotTaken { model: NodeModel, kind: NodeKind },

    #[error("Mode {digits} is not octal digits up to 777, such as 644")]
    ModeNotOctal { digits: u32 },

    #[error(
        "Node '{node}' (model: {model}) requested data_interface_count {requested} but only 0..={max_allowed} data interfaces are supported (reserved: {reserved}, max interface index: {max_index})"
    )]
    DataInterfaceCountUnsupported {
        node: String,
        model: NodeModel,
        requested: u8,
        max_allowed: u8,
        reserved: u8,
        max_index: u8,
    },

    #[error(
        "Node '{node}' (model: {model}) requested data_interface_count {requested} but reserved interface count {reserved} causes interface index overflow"
    )]
    DataInterfaceIndexOverflow {
        node: String,
        model: NodeModel,
        requested: u8,
        reserved: u8,
    },

    #[error("Password does not meet security requirements:\n{}", bulleted(lacking))]
    WeakPassword { lacking: Vec<PasswordRequirement> },

    #[error(
        "SSH key algorithm '{algorithm}' is not supported. Supported algorithms: {supported}",
        supported = SUPPORTED_SSH_KEY_ALGORITHMS
    )]
    UnsupportedSshKeyAlgorithm { algorithm: Algorithm },

    #[error(
        "Tailscale enrollment requires a nonempty Tailscale auth key; OAuth credentials and whitespace are not supported"
    )]
    MalformedTailscaleKey,

    #[error(
        "Username must be {min}–{max} lowercase letters, digits, underscores or hyphens, starting with a letter or underscore",
        min = USERNAME_MIN_LENGTH,
        max = USERNAME_MAX_LENGTH
    )]
    InvalidUsername,

    #[error("Username '{username}' is reserved")]
    ReservedUsername { username: String },

    #[error(
        "ZTP setting '{ztp}' is not supported by model {model}, which allows: {}",
        joined(allowed)
    )]
    UnsupportedZtp {
        ztp: Ztp,
        model: NodeModel,
        allowed: Vec<Ztp>,
    },
}

/// The requirements a password lacks, one bullet per line.
fn bulleted(lacking: &[PasswordRequirement]) -> String {
    lacking
        .iter()
        .map(|requirement| format!("• {requirement}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The ZTP settings a model allows, separated by commas.
fn joined(allowed: &[Ztp]) -> String {
    allowed
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
