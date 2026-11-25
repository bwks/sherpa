use std::io;
use std::path::PathBuf;

use bollard::errors::Error as DockerError;
use thiserror::Error;
use toml::ser::Error as TomlError;

/// A microVM operation's failure.
#[derive(Debug, Error)]
pub enum MicroVmError {
    #[error("Cannot {action} {path}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("Cannot start Cloud Hypervisor for {name}")]
    Spawn {
        name: String,
        #[source]
        source: io::Error,
    },

    #[error("Cloud Hypervisor for {name} did not open its API socket {socket} in time")]
    ApiNotReady { name: String, socket: PathBuf },

    #[error("Cloud Hypervisor API request {endpoint} on {socket} failed: {message}")]
    Api {
        socket: PathBuf,
        endpoint: String,
        message: String,
    },

    #[error("Cloud Hypervisor API request {endpoint} returned {status}: {body}")]
    ApiStatus {
        endpoint: String,
        status: u16,
        body: String,
    },

    #[error("Docker failed to {action} for image {image}")]
    Docker {
        action: &'static str,
        image: String,
        #[source]
        source: DockerError,
    },

    #[error("Image {image} has no ID")]
    ImageWithoutId { image: String },

    #[error("Building the microVM disk for image {image} failed; see {log}")]
    BaseDiskBuild { image: String, log: PathBuf },

    #[error("Building the microVM disk for image {image} did not finish in time; see {log}")]
    BaseDiskTimeout { image: String, log: PathBuf },

    #[error("Cannot write the guest config")]
    GuestConfig(#[source] TomlError),

    #[error("Cannot write the guest config ISO {path}")]
    ConfigIso {
        path: PathBuf,
        #[source]
        source: iso::IsoError,
    },

    #[error("Volume '{volume}' has no container path")]
    Volume { volume: String },

    #[error("Cloud Hypervisor for {name} (pid {pid}) did not stop")]
    NotStopped { name: String, pid: i32 },

    #[error("Cannot stop Cloud Hypervisor for {name} (pid {pid})")]
    Kill {
        name: String,
        pid: i32,
        #[source]
        source: nix::Error,
    },
}
