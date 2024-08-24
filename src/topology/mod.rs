mod connection;
mod device;
mod manifest;

// re-export
pub use crate::topology::connection::Connection;
pub use crate::topology::device::{Device, Interface};
pub use crate::topology::manifest::Manifest;
