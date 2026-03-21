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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tcp_connect_invalid_address_returns_error() {
        let result = tcp_connect("not_an_ip", 8080);
        assert!(result.is_err());
    }

    #[test]
    fn test_tcp_connect_closed_port_returns_false() {
        // Port 1 is almost certainly not listening
        let result = tcp_connect("127.0.0.1", 1);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
}
