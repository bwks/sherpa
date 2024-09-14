use serde_derive::{Deserialize, Serialize};

/// Manifest Connection
#[derive(Debug, Deserialize, Serialize)]
pub struct Connection {
    pub device_a: String,
    pub interface_a: u8,
    pub device_b: String,
    pub interface_b: u8,
}

/// Interfaces Connection Map
// Each device has a loopback assigned from the 127.127.127.0/24 range
// Connections will be created between devices with UDP tunnels in the 10k range.
// Interfaces with no defined connection will be set to 'down' status
// In the XML, source is the remote peer
#[derive(Debug, Deserialize, Serialize)]
pub struct ConnectionMap {
    pub local_id: u8,
    pub local_port: u16,
    pub local_loopback: String,
    pub source_id: u8,
    pub source_port: u16,
    pub source_loopback: String,
}
