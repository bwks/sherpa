use std::io;

use bollard::errors::Error as DockerError;
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

    #[error("Cannot read the node VM's {0}")]
    NodeVmCertificate(&'static str),

    #[error("Cannot make a Docker client for a node VM")]
    NodeVmClient(#[source] DockerError),

    #[error(
        "Cannot copy '{0}' into a node VM's container: the path must be absolute, without '..', and fit a tar header"
    )]
    NodeVmFilePath(String),

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
