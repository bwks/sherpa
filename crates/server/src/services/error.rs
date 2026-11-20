//! Typed failures that services return inside `anyhow::Error`.
//!
//! Transports find a `ServiceError` with [`ServiceError::find`] and map it to their own status
//! codes (REST status, RPC error code, HTML template), instead of matching on error text.

use std::fmt;

use db::{CpuPinTaken, UserChanged};
use shared::error::{CpuPinConflict, MemoryShortfall};

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
    /// Host CPUs the operation would pin are held by another node, stored or being started.
    /// Nothing was started; the same request can succeed once that node lets them go.
    CpuPinsHeld {
        message: String,
        conflict: CpuPinConflict,
    },
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(message)
            | Self::Conflict(message)
            | Self::Invalid(message)
            | Self::InsufficientMemory { message, .. }
            | Self::CpuPinsHeld { message, .. } => write!(f, "{message}"),
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

    /// The error of a failed `db::claim_cpu_pins`: a host CPU the database's trigger refused,
    /// as another node holds it, becomes the same [`ServiceError::CpuPinsHeld`] admission
    /// refuses with.
    #[must_use]
    pub fn from_cpu_pin_claim(error: anyhow::Error) -> anyhow::Error {
        match error.downcast_ref::<CpuPinTaken>() {
            Some(taken) => Self::CpuPinsHeld {
                message: taken.to_string(),
                conflict: CpuPinConflict {
                    host_cpus: vec![taken.host_cpu],
                    requested_role: taken.requested_role,
                    held_role: taken.held_role,
                },
            }
            .into(),
            None => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use shared::error::CpuPinRole;

    #[test]
    fn a_cpu_the_database_refused_to_pin_is_the_pin_refusal() {
        let error = ServiceError::from_cpu_pin_claim(
            CpuPinTaken {
                node_name: "n1".to_owned(),
                host_cpu: 3,
                requested_role: CpuPinRole::Thread,
                held_role: CpuPinRole::Vcpu,
            }
            .into(),
        );
        assert_eq!(
            ServiceError::find(&error),
            Some(&ServiceError::CpuPinsHeld {
                message: "Host CPU 3, which node 'n1' would pin to its QEMU threads, is pinned to \
                          the vCPUs of another node. Try again once that node is destroyed or \
                          redeployed without it"
                    .to_owned(),
                conflict: CpuPinConflict {
                    host_cpus: vec![3],
                    requested_role: CpuPinRole::Thread,
                    held_role: CpuPinRole::Vcpu,
                },
            })
        );
        let other = ServiceError::from_cpu_pin_claim(anyhow!("disk full"));
        assert!(ServiceError::find(&other).is_none());
    }
}
