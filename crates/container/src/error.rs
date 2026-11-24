use std::io;

use thiserror::Error;

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

    #[error("Tailscale gateway stopped after a failed enrollment")]
    GatewayStoppedAfterFailedEnrollment,
}
