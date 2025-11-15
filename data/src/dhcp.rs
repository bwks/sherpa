#[derive(Debug)]
pub struct DhcpLease {
    pub expiry: u64,
    pub mac_address: String,
    pub ipv4_address: String,
    pub hostname: String,
    pub client_id: String,
}
