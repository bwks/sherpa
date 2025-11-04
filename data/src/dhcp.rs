#[derive(Debug)]
pub struct DhcpLease {
    pub expiry: u64,
    pub mac: String,
    pub ip: String,
    pub hostname: String,
    pub client_id: String,
}
