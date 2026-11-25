use std::error;
use std::fmt::{self, Write as _};

use anyhow::{Context, Error, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use shared::error::RpcErrorCode;
use uuid::Uuid;

/// RPC request sent from client to server
#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest {
    pub r#type: String,
    pub id: String,
    pub method: String,
    pub params: serde_json::Value,
}

/// RPC response received from server
#[derive(Debug, Clone, Deserialize)]
pub struct RpcResponse {
    pub id: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<RpcError>,
}

/// RPC error information
#[derive(Debug, Clone, Deserialize)]
pub struct RpcError {
    pub code: RpcErrorCode,
    pub message: String,
    pub context: Option<String>,
}

/// A call the server answered with an error, with its code, so the CLI can choose an exit code
/// without reading the message.
#[derive(Debug)]
pub struct RpcFailure {
    pub code: RpcErrorCode,
    text: String,
}

impl fmt::Display for RpcFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl error::Error for RpcFailure {}

impl RpcRequest {
    /// Create a new RPC request with a generated UUID
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            r#type: "rpc_request".to_string(),
            id: Uuid::new_v4().to_string(),
            method: method.into(),
            params,
        }
    }
}

impl RpcResponse {
    /// The result of the call, parsed as `R`. `failure` says what failed, such as "Lab creation
    /// failed", and starts the error when the server returned one.
    ///
    /// # Errors
    ///
    /// Fails with the server's error, when there is no result, or when it does not parse.
    pub fn into_result<R: DeserializeOwned>(self, failure: &str) -> Result<R> {
        if let Some(error) = self.error {
            return Err(error.into_error(failure));
        }
        let result = self
            .result
            .with_context(|| format!("{failure}: the server sent no result"))?;
        serde_json::from_value(result)
            .with_context(|| format!("{failure}: the server's response does not parse"))
    }
}

impl RpcError {
    /// This error after `failure`, with how to log in again when the token was refused.
    /// The error is an [`RpcFailure`].
    pub fn into_error(self, failure: &str) -> anyhow::Error {
        let mut text = format!("{failure}: {} (code: {})", self.message, self.code);
        if let Some(context) = self.context {
            let _ = write!(text, "\n  Context: {context}");
        }
        if self.code == RpcErrorCode::AuthRequired {
            text.push_str("\n  Your login has expired or is invalid; run: sherpa login");
        }
        Error::new(RpcFailure {
            code: self.code,
            text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(code: RpcErrorCode) -> RpcError {
        RpcError {
            code,
            message: "Not enough memory on the server".to_string(),
            context: None,
        }
    }

    #[test]
    fn user_sessions_011_an_expired_login_says_how_to_log_in_again() {
        let error = error(RpcErrorCode::AuthRequired).into_error("Up failed");
        assert!(matches!(
            error.downcast_ref::<RpcFailure>(),
            Some(RpcFailure {
                code: RpcErrorCode::AuthRequired,
                ..
            })
        ));
    }
}
