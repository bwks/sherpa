use serde::{Deserialize, Serialize};
use std::fmt;

/// JSON-RPC 2.0 error codes
///
/// Standard error codes follow the JSON-RPC 2.0 specification:
/// <https://www.jsonrpc.org/specification#error_object>
///
/// Application-specific codes are in the range -32000 to -32099 as per spec.
///
/// # Examples
///
/// ```
/// use shared::error::RpcErrorCode;
///
/// let code = RpcErrorCode::AuthRequired;
/// assert_eq!(code.code(), -32002);
///
/// // Convert to i32 for serialization
/// let num: i32 = code.into();
/// assert_eq!(num, -32002);
///
/// // Parse from i32
/// let parsed = RpcErrorCode::from(-32002);
/// assert_eq!(parsed, RpcErrorCode::AuthRequired);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpcErrorCode {
    // ============================================================================
    // Standard JSON-RPC 2.0 error codes
    // ============================================================================
    /// Parse error: Invalid JSON was received by the server (-32700)
    ParseError,

    /// Invalid Request: The JSON sent is not a valid Request object (-32600)
    InvalidRequest,

    /// Method not found: The method does not exist / is not available (-32601)
    MethodNotFound,

    /// Invalid params: Invalid method parameter(s) (-32602)
    InvalidParams,

    /// Internal error: Internal JSON-RPC error (-32603)
    InternalError,

    // ============================================================================
    // Application-specific error codes (range: -32000 to -32099)
    // ============================================================================
    /// Server error: General operation failure or unhandled error (-32000)
    ServerError,

    /// Authentication error: Invalid credentials provided (-32001)
    AuthInvalid,

    /// Authentication required: Request requires authentication token (-32002)
    AuthRequired,

    /// Access denied: User lacks permission for requested operation (-32003)
    AccessDenied,

    /// Not found: Requested resource does not exist (-32004)
    NotFound,

    /// Unknown error code: Used when deserializing an unrecognized code
    Unknown(i32),
}

impl RpcErrorCode {
    /// Get the i32 error code value
    ///
    /// # Examples
    ///
    /// ```
    /// use shared::error::RpcErrorCode;
    ///
    /// assert_eq!(RpcErrorCode::AuthRequired.code(), -32002);
    /// assert_eq!(RpcErrorCode::ParseError.code(), -32700);
    /// ```
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
    fn test_error_code_values() {
        assert_eq!(RpcErrorCode::ParseError.code(), -32700);
        assert_eq!(RpcErrorCode::InvalidRequest.code(), -32600);
        assert_eq!(RpcErrorCode::MethodNotFound.code(), -32601);
        assert_eq!(RpcErrorCode::InvalidParams.code(), -32602);
        assert_eq!(RpcErrorCode::InternalError.code(), -32603);
        assert_eq!(RpcErrorCode::ServerError.code(), -32000);
        assert_eq!(RpcErrorCode::AuthInvalid.code(), -32001);
        assert_eq!(RpcErrorCode::AuthRequired.code(), -32002);
        assert_eq!(RpcErrorCode::AccessDenied.code(), -32003);
        assert_eq!(RpcErrorCode::NotFound.code(), -32004);
    }

    #[test]
    fn test_from_i32() {
        assert_eq!(RpcErrorCode::from(-32700), RpcErrorCode::ParseError);
        assert_eq!(RpcErrorCode::from(-32600), RpcErrorCode::InvalidRequest);
        assert_eq!(RpcErrorCode::from(-32601), RpcErrorCode::MethodNotFound);
        assert_eq!(RpcErrorCode::from(-32602), RpcErrorCode::InvalidParams);
        assert_eq!(RpcErrorCode::from(-32603), RpcErrorCode::InternalError);
        assert_eq!(RpcErrorCode::from(-32000), RpcErrorCode::ServerError);
        assert_eq!(RpcErrorCode::from(-32001), RpcErrorCode::AuthInvalid);
        assert_eq!(RpcErrorCode::from(-32002), RpcErrorCode::AuthRequired);
        assert_eq!(RpcErrorCode::from(-32003), RpcErrorCode::AccessDenied);
        assert_eq!(RpcErrorCode::from(-32004), RpcErrorCode::NotFound);
    }

    #[test]
    fn test_from_i32_unknown() {
        let unknown = RpcErrorCode::from(-99999);
        assert_eq!(unknown, RpcErrorCode::Unknown(-99999));
        assert_eq!(unknown.code(), -99999);
    }

    #[test]
    fn test_into_i32() {
        let code: i32 = RpcErrorCode::AuthRequired.into();
        assert_eq!(code, -32002);
    }

    #[test]
    fn test_display() {
        assert_eq!(RpcErrorCode::AuthRequired.to_string(), "-32002");
        assert_eq!(RpcErrorCode::ParseError.to_string(), "-32700");
    }

    #[test]
    fn test_serialize() {
        let code = RpcErrorCode::AuthRequired;
        let json = serde_json::to_string(&code).unwrap();
        assert_eq!(json, "-32002");
    }

    #[test]
    fn test_deserialize() {
        let code: RpcErrorCode = serde_json::from_str("-32002").unwrap();
        assert_eq!(code, RpcErrorCode::AuthRequired);
    }

    #[test]
    fn test_deserialize_unknown() {
        let code: RpcErrorCode = serde_json::from_str("-99999").unwrap();
        assert_eq!(code, RpcErrorCode::Unknown(-99999));
    }
}
