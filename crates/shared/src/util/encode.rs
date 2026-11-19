use anyhow::{Context, Result};

use base64::Engine;
use base64::engine::general_purpose;

#[must_use]
pub fn base64_encode(input: &str) -> String {
    general_purpose::STANDARD.encode(input)
}

/// Base64-encode raw bytes, such as a random key.
#[must_use]
pub fn base64_encode_bytes(input: &[u8]) -> String {
    general_purpose::STANDARD.encode(input)
}

/// Decode base64 `input` to UTF-8 text.
///
/// # Errors
///
/// Fails when `input` is not base64 or does not decode to UTF-8.
pub fn base64_decode(input: &str) -> Result<String> {
    let bytes = general_purpose::STANDARD
        .decode(input)
        .context("Failed to base64 decode")?;
    String::from_utf8(bytes).context("Base64 decoded content is not valid UTF-8")
}

/// Base64-decode to raw bytes, for content that need not be text.
///
/// # Errors
///
/// Fails when `input` is not base64.
pub fn base64_decode_bytes(input: &str) -> Result<Vec<u8>> {
    general_purpose::STANDARD
        .decode(input)
        .context("Failed to base64 decode")
}
