//! Typed failures that services return inside `anyhow::Error`.
//!
//! Transports find a `ServiceError` with [`ServiceError::find`] and map it to their own status
//! codes (REST status, RPC error code, HTML template), instead of matching on error text.

use std::fmt;

use db::UserChanged;
use shared::error::MemoryShortfall;

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
    /// The request itself is malformed, for example a lab ID that is not a lab ID.
    Invalid(String),
    /// The VMs and unikernels the operation would start do not fit in the server's memory now.
    /// Nothing was started; the same request can succeed once other nodes stop.
    InsufficientMemory {
        message: String,
        shortfall: MemoryShortfall,
    },
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(message)
            | Self::Conflict(message)
            | Self::Invalid(message)
            | Self::InsufficientMemory { message, .. } => write!(f, "{message}"),
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

    /// The error of a failed `db::update_user`: a write refused because the user changed since
    /// it was read becomes a [`ServiceError::Conflict`] that tells the caller to try again.
    #[must_use]
    pub fn from_user_write(error: anyhow::Error) -> anyhow::Error {
        match error.downcast_ref::<UserChanged>() {
            Some(changed) => Self::Conflict(changed.to_string()).into(),
            None => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn a_stale_user_write_is_a_conflict() {
        let error = ServiceError::from_user_write(
            UserChanged {
                username: "alice".to_owned(),
            }
            .into(),
        );
        assert!(
            matches!(ServiceError::find(&error), Some(ServiceError::Conflict(message)) if message.contains("try again")),
            "{error:#}"
        );
        let other = ServiceError::from_user_write(anyhow!("disk full"));
        assert!(ServiceError::find(&other).is_none());
    }

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
