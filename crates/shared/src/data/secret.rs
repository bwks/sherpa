//! Secret files the `sherpa` CLI sends for container nodes. They are kept in memory and in
//! RAM-backed files on the server, never in the manifest, the database or logs.
use std::fmt;

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::util::{base64_decode_bytes, base64_encode_bytes};

/// A secret file's contents, base64-encoded. Debug output never shows them.
#[derive(Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(transparent)]
pub struct SecretContent(String);

impl fmt::Debug for SecretContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretContent([REDACTED])")
    }
}

impl SecretContent {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(base64_encode_bytes(bytes))
    }

    pub fn decode(&self) -> Result<Vec<u8>> {
        base64_decode_bytes(&self.0)
    }
}

/// A secret file for a container node, written to `/run/secrets/<name>` in the container.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SecretFile {
    pub name: String,
    /// File mode as octal digits, for example `444`.
    pub mode: u32,
    pub content: SecretContent,
}

/// Request to rewrite a container node's secret files.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeSecretsRequest {
    pub lab_id: String,
    pub node_name: String,
    pub files: Vec<SecretFile>,
}

/// Response to a [`NodeSecretsRequest`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeSecretsResponse {
    pub node_name: String,
    pub files_written: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_round_trips_binary_data() {
        let bytes = [0u8, 159, 146, 150, b'\n'];
        let content = SecretContent::from_bytes(&bytes);
        assert_eq!(content.decode().unwrap(), bytes);
    }

    #[test]
    fn content_is_base64_on_the_wire_and_hidden_from_debug() {
        let file = SecretFile {
            name: "token".to_string(),
            mode: 444,
            content: SecretContent::from_bytes(b"s3cret"),
        };
        let json = serde_json::to_value(&file).unwrap();
        assert_eq!(json["content"], "czNjcmV0");
        let debug = format!("{file:?}");
        assert!(
            !debug.contains("s3cret") && !debug.contains("czNjcmV0"),
            "{debug}"
        );
        assert!(debug.contains("REDACTED"), "{debug}");
    }

    #[test]
    fn invalid_base64_is_an_error() {
        let content: SecretContent =
            serde_json::from_value(serde_json::json!("not base64!")).unwrap();
        assert!(content.decode().is_err());
    }
}
