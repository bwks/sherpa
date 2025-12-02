use super::ZtpMethods;

#[derive(Clone, Debug)]
pub struct ZtpRecord {
    pub device_name: String,
    pub config_file: String,
    pub mac_address: String,
    pub ztp_method: ZtpMethods,
}
