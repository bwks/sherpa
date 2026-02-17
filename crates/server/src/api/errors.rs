use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

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

/// Convert anyhow::Error to ApiError with pattern matching
impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        let err_str = format!("{:#}", err);

        // Pattern match on error messages
        if err_str.contains("not found in database") || err_str.contains("not found") {
            ApiError::not_found("Resource", err_str)
        } else if err_str.contains("Permission denied") || err_str.contains("owned by another user")
        {
            ApiError::forbidden(err_str)
        } else if err_str.contains("Invalid") || err_str.contains("missing") {
            ApiError::bad_request(err_str)
        } else {
            // Log full error chain for debugging
            tracing::error!("Converting anyhow error: {:?}", err);
            ApiError::internal("An unexpected error occurred")
        }
    }
}
