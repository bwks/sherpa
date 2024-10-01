use std::net::TcpStream;
use std::time::Duration;

use anyhow::Result;

/// Check if a TCP connection can be opened.
pub fn tcp_connect(address: &str, port: u16) -> Result<bool> {
    let address_port = format!("{}:{}", address, port).parse()?;
    match TcpStream::connect_timeout(&address_port, Duration::from_millis(100)) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}
