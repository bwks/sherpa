use std::io;

use thiserror::Error;

use crate::network::NetworkDifference;

/// A container operation's failure that a caller tells apart from the others.
#[derive(Debug, Error)]
pub enum ContainerError {
    #[error("Failed to read image archive {path}")]
    ImageArchiveRead {
        path: String,
        #[source]
        source: io::Error,
    },

    #[error("Refusing to remove an unowned Tailscale gateway")]
    UnownedGateway,

    #[error("Tailscale enrollment failed")]
    EnrollmentFailed,

    #[error("Tailscale gateway stopped after a failed enrollment")]
    GatewayStoppedAfterFailedEnrollment,

    #[error("Unable to stop the Tailscale gateway after a failed enrollment: {rollback:#}")]
    GatewayNotStoppedAfterFailedEnrollment {
        #[source]
        enrollment: anyhow::Error,
        rollback: anyhow::Error,
    },

    #[error(
        "Docker network '{name}' already exists and is not the lab's: {}. It was left in place; \
         remove it with 'docker network rm {name}' if nothing uses it",
        joined(differences)
    )]
    NetworkDiffers {
        name: String,
        differences: Vec<NetworkDifference>,
    },
}

/// The differences of a network, separated by semicolons.
fn joined(differences: &[NetworkDifference]) -> String {
    differences
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
