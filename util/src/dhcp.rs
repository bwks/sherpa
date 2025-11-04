use reqwest;

use data::{Config, DhcpLease};
use konst::{DHCP_LEASES_FILE, DHCP_URI_DIR, HTTP_PORT, SHERPA_MANAGEMENT_VM_IPV4_INDEX};

pub async fn get_dhcp_leases(config: &Config) -> Vec<DhcpLease> {
    let url = format!(
        "http://{}:{}/{}/{}",
        &config
            .management_prefix_ipv4
            .nth(SHERPA_MANAGEMENT_VM_IPV4_INDEX)
            .unwrap(),
        HTTP_PORT,
        DHCP_URI_DIR,
        DHCP_LEASES_FILE,
    );
    // Attempt to fetch; if it fails, supply empty string instead
    match reqwest::get(url).await {
        Ok(response) => {
            let body = response.text().await.unwrap_or_default();
            let leases: Vec<DhcpLease> = body
                .lines()
                .filter_map(|line| {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    if fields.len() == 5 {
                        Some(DhcpLease {
                            expiry: fields[0].parse().unwrap_or(0),
                            mac: fields[1].into(),
                            ip: fields[2].into(),
                            hostname: fields[3].into(),
                            client_id: fields[4].into(),
                        })
                    } else {
                        None
                    }
                })
                .collect();
            leases
        }
        Err(_) => {
            println!("DHCP server not ready yet"); // Ignore error, fallback to empty string (or default content)
            vec![]
        }
    }
}
