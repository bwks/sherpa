//! Docker API error responses that callers treat as outcomes rather than failures.

use bollard::errors::Error as DockerError;

/// Whether Docker answered 404: the container, network, volume or image does not exist.
pub(crate) fn is_missing(error: &DockerError) -> bool {
    matches!(
        error,
        DockerError::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

/// Whether Docker answered 409: something with that name already exists.
pub(crate) fn is_conflict(error: &DockerError) -> bool {
    matches!(
        error,
        DockerError::DockerResponseServerError {
            status_code: 409,
            ..
        }
    )
}
