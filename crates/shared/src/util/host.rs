use std::net::Ipv4Addr;

use anyhow::{Context, Result, anyhow};
#[cfg(feature = "netinfo")]
use getifaddrs::{Address, Interfaces, getifaddrs};

/// Returns the short hostname of the machine.
pub fn get_hostname() -> Result<String> {
    let output = std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .context("Failed to execute 'hostname -s'")?;

    if !output.status.success() {
        return Err(anyhow!("hostname -s exited with status {}", output.status));
    }

    let hostname = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if hostname.is_empty() {
        return Err(anyhow!("hostname -s returned empty output"));
    }

    Ok(hostname)
}

/// Returns the fully qualified domain name (FQDN) of the machine.
///
/// Returns `None` if the FQDN cannot be determined or is the same as the short hostname.
pub fn get_fqdn() -> Option<String> {
    let output = std::process::Command::new("hostname")
        .arg("--fqdn")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let fqdn = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if fqdn.is_empty() {
        return None;
    }

    Some(fqdn)
}

/// Returns all non-loopback IPv4 addresses assigned to network interfaces on the host.
#[cfg(feature = "netinfo")]
pub fn get_non_loopback_ipv4_addresses() -> Result<Vec<Ipv4Addr>> {
    let interfaces = getifaddrs()
        .context("Failed to enumerate network interfaces")?
        .collect::<Interfaces>();

    let mut addresses = Vec::new();
    for (_index, interface) in interfaces {
        for address in interface.address.iter().flatten() {
            if let Address::V4(..) = address
                && let Some(ip_addr) = address.ip_addr()
                && let Ok(ipv4) = ip_addr.to_string().parse::<Ipv4Addr>()
                && !ipv4.is_loopback()
            {
                addresses.push(ipv4);
            }
        }
    }

    Ok(addresses)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_hostname_returns_non_empty() {
        let hostname = get_hostname().unwrap();
        assert!(!hostname.is_empty());
        assert!(!hostname.contains('\n'));
    }

    #[test]
    fn test_get_fqdn_returns_non_empty_if_some() {
        if let Some(fqdn) = get_fqdn() {
            assert!(!fqdn.is_empty());
            assert!(!fqdn.contains('\n'));
        }
    }

    #[cfg(feature = "netinfo")]
    #[test]
    fn test_get_non_loopback_ipv4_addresses() {
        let addrs = get_non_loopback_ipv4_addresses().unwrap();
        for addr in &addrs {
            assert!(!addr.is_loopback());
        }
    }
}
