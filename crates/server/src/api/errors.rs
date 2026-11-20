use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::Value;
use shared::error::{CpuPinConflict, MemoryShortfall};

use crate::services::error::ServiceError;

/// Structured API error with consistent JSON format
#[derive(Debug)]
pub enum ApiError {
    /// 401 Unauthorized - Missing or invalid authentication
    Unauthorized { message: String },

    /// 403 Forbidden - Valid auth but insufficient permissions
    Forbidden { message: String },

    /// 404 Not Found - Resource doesn't exist
    NotFound { resource: String, message: String },

    /// 400 Bad Request - Invalid input
    BadRequest { message: String },

    /// 409 Conflict - Allowed, but the current state forbids the operation
    Conflict { message: String },

    /// 507 Insufficient Storage - The nodes to start do not fit in the server's memory now.
    /// Its own status, so a client can tell it from a conflict and retry later; not 503, which
    /// proxies and monitors read as the server being down.
    InsufficientMemory {
        message: String,
        shortfall: MemoryShortfall,
    },

    /// 409 Conflict - Host CPUs the nodes to start would pin are held by another node. A
    /// conflict with its own `code`, so a client can tell it from other conflicts and retry
    /// once the CPUs are let go.
    CpuPinsHeld {
        message: String,
        conflict: CpuPinConflict,
    },

    /// 500 Internal Server Error - Server-side errors
    InternalError { message: String },
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<String>,
    /// Structured detail for codes that define it: the `MemoryShortfall` of
    /// `INSUFFICIENT_MEMORY` and the `CpuPinConflict` of `CPU_PINS_HELD`.
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl ApiError {
    /// Create an Unauthorized error
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::Unauthorized {
            message: msg.into(),
        }
    }

    /// Create a Forbidden error
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::Forbidden {
            message: msg.into(),
        }
    }

    /// Create a `NotFound` error
    pub fn not_found(resource: impl Into<String>, msg: impl Into<String>) -> Self {
        Self::NotFound {
            resource: resource.into(),
            message: msg.into(),
        }
    }

    /// Create a Conflict error
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict {
            message: msg.into(),
        }
    }

    /// Create a `BadRequest` error
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest {
            message: msg.into(),
        }
    }

    /// Create an `InternalError`
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::InternalError {
            message: msg.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        if let ApiError::InternalError { message } = &self {
            // Log internal errors with context
            tracing::error!("Internal API error: {}", message);
        }
        (status, Json(ErrorResponse::from(self))).into_response()
    }
}

impl ApiError {
    /// The HTTP status of this error.
    fn status(&self) -> StatusCode {
        match self {
            ApiError::Unauthorized { .. } => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden { .. } => StatusCode::FORBIDDEN,
            ApiError::NotFound { .. } => StatusCode::NOT_FOUND,
            ApiError::BadRequest { .. } => StatusCode::BAD_REQUEST,
            ApiError::Conflict { .. } | ApiError::CpuPinsHeld { .. } => StatusCode::CONFLICT,
            ApiError::InsufficientMemory { .. } => StatusCode::INSUFFICIENT_STORAGE,
            ApiError::InternalError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The error's `code` in the response body.
    fn code(&self) -> &'static str {
        match self {
            ApiError::Unauthorized { .. } => "UNAUTHORIZED",
            ApiError::Forbidden { .. } => "FORBIDDEN",
            ApiError::NotFound { .. } => "NOT_FOUND",
            ApiError::BadRequest { .. } => "BAD_REQUEST",
            ApiError::Conflict { .. } => "CONFLICT",
            ApiError::InsufficientMemory { .. } => "INSUFFICIENT_MEMORY",
            ApiError::CpuPinsHeld { .. } => "CPU_PINS_HELD",
            ApiError::InternalError { .. } => "INTERNAL_ERROR",
        }
    }
}

impl From<ApiError> for ErrorResponse {
    fn from(err: ApiError) -> Self {
        let code = err.code();
        let (message, details, data) = match err {
            ApiError::Unauthorized { message } => {
                ("Authentication required".to_string(), Some(message), None)
            }
            ApiError::Forbidden { message } => ("Access denied".to_string(), Some(message), None),
            ApiError::NotFound { resource, message } => {
                (format!("{resource} not found"), Some(message), None)
            }
            ApiError::BadRequest { message } => {
                ("Invalid request".to_string(), Some(message), None)
            }
            ApiError::Conflict { message } => (message.clone(), Some(message), None),
            ApiError::InsufficientMemory { message, shortfall } => {
                (message, None, serde_json::to_value(shortfall).ok())
            }
            ApiError::CpuPinsHeld { message, conflict } => {
                (message, None, serde_json::to_value(conflict).ok())
            }
            // Don't expose internal details to client
            ApiError::InternalError { .. } => {
                ("An internal error occurred".to_string(), None, None)
            }
        };
        ErrorResponse {
            error: ErrorDetail {
                code,
                message,
                details,
                data,
            },
        }
    }
}

/// The JSON of a streaming operation's SSE `error` event: the error's message chain, and for a
/// typed service refusal the `code` and `data` its REST error would have, since the stream's
/// HTTP status was sent before the operation failed.
pub fn stream_error_event(error: &anyhow::Error) -> Value {
    let mut event = serde_json::json!({ "error": format!("{error:#}") });
    if let Some(refusal) = ServiceError::find(error) {
        let api_error = ApiError::from(refusal);
        event["code"] = Value::from(api_error.code());
        if let Some(data) = ErrorResponse::from(api_error).error.data {
            event["data"] = data;
        }
    }
    event
}

/// Convert a typed service refusal to `ApiError`: 400, 403, 404, 409 (a conflict, or CPU pins
/// held) or 507.
impl From<&ServiceError> for ApiError {
    fn from(err: &ServiceError) -> Self {
        match err {
            ServiceError::Denied(_) => ApiError::forbidden(err.to_string()),
            ServiceError::NotFound(message) => ApiError::not_found("Resource", message.clone()),
            ServiceError::Conflict(message) => ApiError::conflict(message.clone()),
            ServiceError::Invalid(message) => ApiError::bad_request(message.clone()),
            ServiceError::InsufficientMemory { message, shortfall } => {
                ApiError::InsufficientMemory {
                    message: message.clone(),
                    shortfall: *shortfall,
                }
            }
            ServiceError::CpuPinsHeld { message, conflict } => ApiError::CpuPinsHeld {
                message: message.clone(),
                conflict: conflict.clone(),
            },
        }
    }
}

impl From<ServiceError> for ApiError {
    fn from(err: ServiceError) -> Self {
        ApiError::from(&err)
    }
}

/// Convert `anyhow::Error` to `ApiError`
///
/// Authorization failures carry a typed `ServiceError` and map to 403 or 404. Other errors
/// still fall back to matching on the error text.
impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        if let Some(access) = ServiceError::find(&err) {
            return ApiError::from(access);
        }

        let err_str = format!("{err:#}");

        // Pattern match on error messages
        if err_str.contains("not found in database") || err_str.contains("not found") {
            ApiError::not_found("Resource", err_str)
        } else if err_str.contains("Invalid") || err_str.contains("missing") {
            ApiError::bad_request(err_str)
        } else {
            // Log full error chain for debugging
            tracing::error!("Converting anyhow error: {:?}", err);
            ApiError::internal("An unexpected error occurred")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use shared::error::{CpuPinConflict, CpuPinRole, MemoryShortfall};

    #[tokio::test]
    async fn test_internal_error_hides_details() {
        let err = ApiError::internal("secret db password leaked");
        let response = err.into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "INTERNAL_ERROR");
        // Details should be None (not exposed to client)
        assert!(json["error"]["details"].is_null());
    }

    #[test]
    fn access_denied_errors_convert_to_forbidden_through_context() {
        let err = anyhow::Error::new(ServiceError::Denied("not yours".to_owned()))
            .context("Failed to destroy lab");
        let api_err: ApiError = err.into();
        assert_eq!(api_err.into_response().status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn conflict_errors_convert_to_conflict() {
        let err = anyhow::Error::new(ServiceError::Conflict("owns labs".to_owned()))
            .context("Failed to delete user");
        let api_err: ApiError = err.into();
        assert_eq!(api_err.into_response().status(), StatusCode::CONFLICT);
    }

    #[test]
    fn access_not_found_errors_convert_to_not_found() {
        let err = anyhow::Error::new(ServiceError::NotFound("Lab not found: x".to_owned()));
        let api_err: ApiError = err.into();
        assert_eq!(api_err.into_response().status(), StatusCode::NOT_FOUND);
    }

    fn memory_refusal() -> anyhow::Error {
        anyhow::Error::new(ServiceError::InsufficientMemory {
            message: "Not enough memory on the server".to_owned(),
            shortfall: MemoryShortfall {
                requested_mib: 8192,
                available_mib: 2048,
                numa_node: Some(1),
            },
        })
        .context("Memory admission failed")
    }

    #[tokio::test]
    async fn a_memory_refusal_is_507_with_its_code_and_shortfall() {
        let api_err: ApiError = memory_refusal().into();
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::INSUFFICIENT_STORAGE);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "INSUFFICIENT_MEMORY");
        assert_eq!(json["error"]["message"], "Not enough memory on the server");
        assert_eq!(
            json["error"]["data"],
            serde_json::json!({ "requested_mib": 8192, "available_mib": 2048, "numa_node": 1 })
        );
    }

    #[test]
    fn a_streamed_memory_refusal_carries_its_code_and_shortfall() {
        let event = stream_error_event(&memory_refusal());
        assert_eq!(event["code"], "INSUFFICIENT_MEMORY");
        assert_eq!(event["data"]["available_mib"], 2048);
        assert_eq!(
            event["error"],
            "Memory admission failed: Not enough memory on the server"
        );
    }

    fn pin_refusal() -> anyhow::Error {
        anyhow::Error::new(ServiceError::CpuPinsHeld {
            message: "Host CPU 2 is held".to_owned(),
            conflict: CpuPinConflict {
                host_cpus: vec![2],
                requested_role: CpuPinRole::Vcpu,
                held_role: CpuPinRole::Thread,
            },
        })
        .context("CPU pinning validation failed")
    }

    #[tokio::test]
    async fn a_pin_refusal_is_409_with_its_code_and_conflict() {
        let api_err: ApiError = pin_refusal().into();
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "CPU_PINS_HELD");
        assert_eq!(json["error"]["message"], "Host CPU 2 is held");
        assert_eq!(
            json["error"]["data"],
            serde_json::json!({ "host_cpus": [2], "requested_role": "vcpu", "held_role": "thread" })
        );
    }

    #[test]
    fn a_streamed_pin_refusal_carries_its_code_and_conflict() {
        let event = stream_error_event(&pin_refusal());
        assert_eq!(event["code"], "CPU_PINS_HELD");
        assert_eq!(event["data"]["host_cpus"], serde_json::json!([2]));
    }
}
