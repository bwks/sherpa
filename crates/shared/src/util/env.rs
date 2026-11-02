use std::env;

/// Get server URL from environment variable
///
/// Checks for SHERPA_SERVER_URL environment variable
pub fn get_server_url() -> Option<String> {
    env::var("SHERPA_SERVER_URL").ok()
}
