use serde::{Deserialize, Serialize};
use std::fmt;

/// JSON-RPC 2.0 error codes
///
/// Standard error codes follow the JSON-RPC 2.0 specification:
/// <https://www.jsonrpc.org/specification#error_object>
///
/// Application-specific codes are in the range -32000 to -32099 as per spec.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpcErrorCode {
    ParseError,

    InvalidRequest,

    MethodNotFound,

    InvalidParams,

    InternalError,

    ServerError,

    AuthInvalid,

    AuthRequired,

    AccessDenied,

    NotFound,

    InsufficientMemory,

    CpuPinsHeld,

    Unknown(i32),
}

impl RpcErrorCode {
    /// Get the i32 error code value
    ///
    #[must_use]
    pub fn code(&self) -> i32 {
        match self {
            Self::ParseError => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::InternalError => -32603,
            Self::ServerError => -32000,
            Self::AuthInvalid => -32001,
            Self::AuthRequired => -32002,
            Self::AccessDenied => -32003,
            Self::NotFound => -32004,
            Self::InsufficientMemory => -32005,
            Self::CpuPinsHeld => -32006,
            Self::Unknown(code) => *code,
        }
    }
}

impl fmt::Display for RpcErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

impl From<RpcErrorCode> for i32 {
    fn from(code: RpcErrorCode) -> Self {
        code.code()
    }
}

impl From<i32> for RpcErrorCode {
    fn from(code: i32) -> Self {
        match code {
            -32700 => Self::ParseError,
            -32600 => Self::InvalidRequest,
            -32601 => Self::MethodNotFound,
            -32602 => Self::InvalidParams,
            -32603 => Self::InternalError,
            -32000 => Self::ServerError,
            -32001 => Self::AuthInvalid,
            -32002 => Self::AuthRequired,
            -32003 => Self::AccessDenied,
            -32004 => Self::NotFound,
            -32005 => Self::InsufficientMemory,
            -32006 => Self::CpuPinsHeld,
            _ => Self::Unknown(code),
        }
    }
}

// Serialize as i32 for JSON-RPC spec compliance
impl Serialize for RpcErrorCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_i32(self.code())
    }
}

// Deserialize from i32
impl<'de> Deserialize<'de> for RpcErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let code = i32::deserialize(deserializer)?;
        Ok(Self::from(code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_errors_005_an_error_code_is_sent_as_its_number_and_an_unknown_number_is_kept() {
        assert_eq!(
            serde_json::to_string(&RpcErrorCode::AuthRequired).unwrap(),
            "-32002"
        );
        let code: RpcErrorCode = serde_json::from_str("-32002").unwrap();
        assert_eq!(code, RpcErrorCode::AuthRequired);
        let unknown: RpcErrorCode = serde_json::from_str("-99999").unwrap();
        assert_eq!(unknown, RpcErrorCode::Unknown(-99999));
        assert_eq!(serde_json::to_string(&unknown).unwrap(), "-99999");
    }
}
