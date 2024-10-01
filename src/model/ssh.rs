use std::fmt;
use std::str::FromStr;

use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SshKeyAlgorithms {
    #[default]
    SshEd25519,
    SshRsa,
}

impl fmt::Display for SshKeyAlgorithms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SshKeyAlgorithms::SshEd25519 => write!(f, "ssh-ed25519"),
            SshKeyAlgorithms::SshRsa => write!(f, "ssh-rsa"),
        }
    }
}

#[derive(Debug)]
pub struct SshKeyAlgorithmsError;

impl FromStr for SshKeyAlgorithms {
    type Err = SshKeyAlgorithmsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ssh-ed25519" => Ok(SshKeyAlgorithms::SshEd25519),
            "ssh-rsa" => Ok(SshKeyAlgorithms::SshRsa),
            _ => Err(SshKeyAlgorithmsError),
        }
    }
}

impl fmt::Display for SshKeyAlgorithmsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Invalid SSH key algorithm variant")
    }
}

impl std::error::Error for SshKeyAlgorithmsError {}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SshPublicKey {
    pub algorithm: SshKeyAlgorithms,
    pub key: String,
    pub comment: Option<String>,
}
