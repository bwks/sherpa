use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::Value;
use shared::error::{CpuPinConflict, MemoryShortfall};

use crate::services::error::{ServiceCategory, ServiceError, Unexpected, caller_message};

/// Structured API error with consistent JSON format
#[derive(Debug)]
pub enum ApiError {
    Unauthorized {
        message: String,
    },

    Forbidden {
        message: String,
    },

    NotFound {
        resource: String,
        message: String,
    },

    BadRequest {
        message: String,
    },

    Conflict {
        message: String,
    },

    InsufficientMemory {
        message: String,
        shortfall: MemoryShortfall,
    },

    CpuPinsHeld {
        message: String,
        conflict: CpuPinConflict,
    },

    InternalError(Unexpected),
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

    /// Create an `InternalError` for `error`, which `failure` describes: the error is logged
    /// with its chain and a reference, and only the reference is answered.
    pub fn internal(error: impl Into<anyhow::Error>, failure: &str) -> Self {
        Self::InternalError(Unexpected::log(&error.into(), failure))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
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
            ApiError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
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
            ApiError::InternalError(_) => "INTERNAL_ERROR",
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
            ApiError::InternalError(unexpected) => (unexpected.to_string(), None, None),
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

/// The JSON of a streaming operation's SSE `error` event: what the caller is told of the error,
/// and for a typed service refusal the `code` and `data` its REST error would have, since the
/// stream's HTTP status was sent before the operation failed.
pub fn stream_error_event(error: &anyhow::Error) -> Value {
    let mut event =
        serde_json::json!({ "error": caller_message(error, "Streamed API operation failed") });
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
            ServiceError::InsufficientMemory { shortage } => ApiError::InsufficientMemory {
                message: shortage.to_string(),
                shortfall: shortage.shortfall(),
            },
            ServiceError::CpuPinsHeld { message, conflict } => ApiError::CpuPinsHeld {
                message: message.clone(),
                conflict: conflict.clone(),
            },
            refusal => match refusal.category() {
                Some(ServiceCategory::Denied) => ApiError::forbidden(refusal.message()),
                Some(ServiceCategory::NotFound) => {
                    ApiError::not_found("Resource", refusal.message())
                }
                Some(ServiceCategory::Conflict) => ApiError::conflict(refusal.message()),
                Some(ServiceCategory::Invalid) => ApiError::bad_request(refusal.message()),
                None => ApiError::internal(refusal.clone(), "API request failed"),
            },
        }
    }
}

impl From<ServiceError> for ApiError {
    fn from(err: ServiceError) -> Self {
        ApiError::from(&err)
    }
}

/// Convert `anyhow::Error` to `ApiError`: the `ServiceError` in its chain by its kind, and any
/// other error as an `InternalError`.
impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        match ServiceError::find(&err) {
            Some(refusal) => ApiError::from(refusal),
            None => ApiError::internal(err, "API request failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Error;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use shared::error::{CpuPinConflict, CpuPinRole};

    use crate::validate::MemoryShortage;

    /// The reference in `message`, which must be the answer to an unexpected error.
    fn reference_of(message: &str) -> &str {
        let reference = message
            .strip_prefix("internal error (ref ")
            .and_then(|rest| rest.strip_suffix("); see the server log"))
            .unwrap_or_else(|| panic!("not an unexpected error's answer: {message}"));
        assert!(
            reference.len() == 8 && reference.chars().all(|c| c.is_ascii_hexdigit()),
            "{reference}"
        );
        reference
    }

    #[tokio::test]
    async fn api_errors_002_an_internal_error_over_rest_is_answered_with_a_ref_and_no_detail() {
        let err = anyhow::anyhow!("permission denied on /var/lib/sherpa/secret.key")
            .context("Failed to read lab key");
        let response = ApiError::from(err).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "INTERNAL_ERROR");
        reference_of(json["error"]["message"].as_str().unwrap());
        assert!(json["error"]["details"].is_null(), "{json}");
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("secret.key"), "{text}");
        assert!(!text.contains("Failed to read lab key"), "{text}");
    }

    #[tokio::test]
    async fn api_errors_019_a_rest_refusal_of_a_known_kind_is_answered_with_its_own_message() {
        let err = Error::new(ServiceError::NotFound(
            "Node 'r9' not found in lab 'abc12345'".to_owned(),
        ))
        .context("Failed to stop node");
        let response = ApiError::from(err).into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "NOT_FOUND");
        assert_eq!(
            json["error"]["details"],
            "Node 'r9' not found in lab 'abc12345'"
        );
    }

    #[test]
    fn api_errors_021_a_streamed_rest_failure_of_no_known_kind_is_a_ref_without_its_chain() {
        let err = anyhow::anyhow!("Docker responded with status code 500: no such image")
            .context("Failed to pull image");
        let event = stream_error_event(&err);
        reference_of(event["error"].as_str().unwrap());
        assert!(event.get("code").is_none(), "{event}");
        let text = event.to_string();
        assert!(!text.contains("Docker"), "{text}");
        assert!(!text.contains("Failed to pull image"), "{text}");
    }

    #[test]
    fn access_denied_errors_convert_to_forbidden_through_context() {
        let err = Error::new(ServiceError::Denied("not yours".to_owned()))
            .context("Failed to destroy lab");
        let api_err: ApiError = err.into();
        assert_eq!(api_err.into_response().status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn conflict_errors_convert_to_conflict() {
        let err = Error::new(ServiceError::Conflict("owns labs".to_owned()))
            .context("Failed to delete user");
        let api_err: ApiError = err.into();
        assert_eq!(api_err.into_response().status(), StatusCode::CONFLICT);
    }

    #[test]
    fn access_not_found_errors_convert_to_not_found() {
        let err = Error::new(ServiceError::NotFound("Lab not found: x".to_owned()));
        let api_err: ApiError = err.into();
        assert_eq!(api_err.into_response().status(), StatusCode::NOT_FOUND);
    }

    fn shortage() -> MemoryShortage {
        MemoryShortage::NumaNode {
            numa_node: 1,
            requested: vec!["r1".to_owned()],
            needed_mib: 8192,
            left_mib: 2048,
            node_mib: 16384,
            hugepages_mib: 0,
            reserve_share_mib: 1024,
            overcommit_percent: 100,
            limit_mib: 15360,
            held_mib: 13312,
            holders: Vec::new(),
        }
    }

    fn memory_refusal() -> anyhow::Error {
        Error::new(ServiceError::InsufficientMemory {
            shortage: shortage(),
        })
        .context("Memory admission failed")
    }

    #[tokio::test]
    async fn a_memory_refusal_is_507_with_its_code_and_shortfall() {
        let api_err: ApiError = memory_refusal().into();
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::INSUFFICIENT_STORAGE);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "INSUFFICIENT_MEMORY");
        assert_eq!(json["error"]["message"], shortage().to_string());
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
        assert_eq!(event["error"], shortage().to_string());
    }

    fn pin_refusal() -> anyhow::Error {
        Error::new(ServiceError::CpuPinsHeld {
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
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
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
