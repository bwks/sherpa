//! Typed failures that services return inside `anyhow::Error`.
//!
//! Transports find a `ServiceError` with [`ServiceError::find`] and map it to their own status
//! codes (REST status, RPC error code, HTML template), instead of matching on error text.

use std::fmt;

/// A service refused a request for a reason the caller can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    /// The resource does not exist.
    NotFound(String),
    /// The caller is authenticated but not allowed to perform the operation.
    Denied(String),
    /// The caller is allowed, but the current state forbids the operation, for example
    /// deleting a user who still owns labs.
    Conflict(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(message) | Self::Conflict(message) => write!(f, "{message}"),
            Self::Denied(message) => write!(f, "Access denied: {message}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl ServiceError {
    /// Find a `ServiceError` anywhere in an error chain.
    pub fn find(error: &anyhow::Error) -> Option<&ServiceError> {
        error.downcast_ref::<ServiceError>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn service_errors_survive_added_context() {
        let error = anyhow::Error::new(ServiceError::NotFound("Lab not found: lab1".to_owned()))
            .context("Failed to destroy lab");
        assert_eq!(
            ServiceError::find(&error),
            Some(&ServiceError::NotFound("Lab not found: lab1".to_owned()))
        );
        assert!(ServiceError::find(&anyhow!("Lab not found: lab1")).is_none());
    }

    #[test]
    fn conflict_messages_are_shown_as_written() {
        let error = ServiceError::Conflict("User 'alice' owns 1 lab".to_owned());
        assert_eq!(error.to_string(), "User 'alice' owns 1 lab");
    }
}
