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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_ed25519() {
        assert_eq!(SshKeyAlgorithms::SshEd25519.to_string(), "ssh-ed25519");
    }

    #[test]
    fn test_display_rsa() {
        assert_eq!(SshKeyAlgorithms::SshRsa.to_string(), "ssh-rsa");
    }

    #[test]
    fn test_from_str_ed25519() {
        let alg: SshKeyAlgorithms = "ssh-ed25519".parse().unwrap();
        assert!(matches!(alg, SshKeyAlgorithms::SshEd25519));
    }

    #[test]
    fn test_from_str_rsa() {
        let alg: SshKeyAlgorithms = "ssh-rsa".parse().unwrap();
        assert!(matches!(alg, SshKeyAlgorithms::SshRsa));
    }

    #[test]
    fn test_from_str_invalid_returns_err() {
        assert!("ecdsa-sha2-nistp256".parse::<SshKeyAlgorithms>().is_err());
    }

    #[test]
    fn test_error_display() {
        assert_eq!(
            SshKeyAlgorithmsError.to_string(),
            "Invalid SSH key algorithm variant"
        );
    }

    #[test]
    fn test_default_is_ed25519() {
        assert!(matches!(
            SshKeyAlgorithms::default(),
            SshKeyAlgorithms::SshEd25519
        ));
    }

    #[test]
    fn test_serde_round_trip() {
        let original = SshPublicKey {
            algorithm: SshKeyAlgorithms::SshEd25519,
            key: "AAAAC3NzaC1lZDI1NTE5AAAA".to_string(),
            comment: Some("user@host".to_string()),
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: SshPublicKey = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.key, original.key);
        assert_eq!(deserialized.comment, original.comment);
    }
}
