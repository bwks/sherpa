use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

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

    /// 500 Internal Server Error - Server-side errors
    InternalError { message: String },
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<String>,
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

    /// Create a NotFound error
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

    /// Create a BadRequest error
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest {
            message: msg.into(),
        }
    }

    /// Create an InternalError
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::InternalError {
            message: msg.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, details) = match self {
            ApiError::Unauthorized { message } => (
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED".to_string(),
                "Authentication required".to_string(),
                Some(message),
            ),
            ApiError::Forbidden { message } => (
                StatusCode::FORBIDDEN,
                "FORBIDDEN".to_string(),
                "Access denied".to_string(),
                Some(message),
            ),
            ApiError::NotFound { resource, message } => (
                StatusCode::NOT_FOUND,
                "NOT_FOUND".to_string(),
                format!("{} not found", resource),
                Some(message),
            ),
            ApiError::BadRequest { message } => (
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST".to_string(),
                "Invalid request".to_string(),
                Some(message),
            ),
            ApiError::Conflict { message } => (
                StatusCode::CONFLICT,
                "CONFLICT".to_string(),
                message.clone(),
                Some(message),
            ),
            ApiError::InternalError { message } => {
                // Log internal errors with context
                tracing::error!("Internal API error: {}", message);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR".to_string(),
                    "An internal error occurred".to_string(),
                    None, // Don't expose internal details to client
                )
            }
        };

        let body = ErrorResponse {
            error: ErrorDetail {
                code,
                message,
                details,
            },
        };

        (status, Json(body)).into_response()
    }
}

/// Convert a typed service refusal to ApiError: 403, 404 or 409.
impl From<&ServiceError> for ApiError {
    fn from(err: &ServiceError) -> Self {
        match err {
            ServiceError::Denied(_) => ApiError::forbidden(err.to_string()),
            ServiceError::NotFound(message) => ApiError::not_found("Resource", message.clone()),
            ServiceError::Conflict(message) => ApiError::conflict(message.clone()),
        }
    }
}

impl From<ServiceError> for ApiError {
    fn from(err: ServiceError) -> Self {
        ApiError::from(&err)
    }
}

/// Convert anyhow::Error to ApiError
///
/// Authorization failures carry a typed `ServiceError` and map to 403 or 404. Other errors
/// still fall back to matching on the error text.
impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        if let Some(access) = ServiceError::find(&err) {
            return ApiError::from(access);
        }

        let err_str = format!("{:#}", err);

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

    #[test]
    fn test_unauthorized_status_code() {
        let err = ApiError::unauthorized("bad token");
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_forbidden_status_code() {
        let err = ApiError::forbidden("not allowed");
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn test_not_found_status_code() {
        let err = ApiError::not_found("Lab", "lab-123 not found");
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_bad_request_status_code() {
        let err = ApiError::bad_request("missing field");
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_internal_error_status_code() {
        let err = ApiError::internal("db crashed");
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_unauthorized_json_body() {
        let err = ApiError::unauthorized("invalid token");
        let response = err.into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "UNAUTHORIZED");
        assert_eq!(json["error"]["message"], "Authentication required");
        assert_eq!(json["error"]["details"], "invalid token");
    }

    #[tokio::test]
    async fn test_not_found_json_body() {
        let err = ApiError::not_found("Lab", "lab-123 missing");
        let response = err.into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "NOT_FOUND");
        assert_eq!(json["error"]["message"], "Lab not found");
        assert_eq!(json["error"]["details"], "lab-123 missing");
    }

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
    fn test_anyhow_not_found_conversion() {
        let err = anyhow::anyhow!("Lab not found in database");
        let api_err: ApiError = err.into();
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
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

    #[test]
    fn permission_text_alone_is_not_treated_as_an_access_decision() {
        let err = anyhow::anyhow!("Permission denied (os error 13)");
        let api_err: ApiError = err.into();
        assert_eq!(
            api_err.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_anyhow_invalid_conversion() {
        let err = anyhow::anyhow!("Invalid input format");
        let api_err: ApiError = err.into();
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_anyhow_generic_error_conversion() {
        let err = anyhow::anyhow!("something unexpected happened");
        let api_err: ApiError = err.into();
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
