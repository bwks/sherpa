//! Server tables: its status, trusted certificates, a backup and a user's SSH keys.

use shared::data::BackupResponse;
use shared::util::{render_table, ssh_key_fingerprint};

const CERTIFICATE_HEADERS: [&str; 3] = ["Server", "Subject", "Valid Until"];
const SSH_KEY_HEADERS: [&str; 4] = ["#", "Type", "Fingerprint", "Comment"];
const SERVER_STATUS_HEADERS: [&str; 3] = ["Server", "Status", "TLS"];
const BACKUP_HEADERS: [&str; 2] = ["Backup", "Details"];

/// Certificate information for table display
pub struct CertificateTableInfo {
    pub server: String,
    pub subject: String,
    pub valid_until: String,
}

/// Renders a table of trusted certificates
#[must_use]
pub fn render_certificates_table(certs: &[CertificateTableInfo]) -> String {
    let rows: Vec<[String; 3]> = certs.iter().map(certificate_cells).collect();
    render_table(
        Some("Trusted Server Certificates"),
        CERTIFICATE_HEADERS,
        &rows,
        &[],
    )
}

fn certificate_cells(cert: &CertificateTableInfo) -> [String; 3] {
    [
        cert.server.clone(),
        cert.subject.clone(),
        cert.valid_until.clone(),
    ]
}

/// Render a user's SSH public keys with their SHA256 fingerprints
#[must_use]
pub fn render_ssh_keys_table(username: &str, keys: &[String]) -> String {
    let rows: Vec<[String; 4]> = keys
        .iter()
        .enumerate()
        .map(|(index, key)| ssh_key_cells(index + 1, key))
        .collect();
    render_table(
        Some(&format!("SSH Keys for {username}")),
        SSH_KEY_HEADERS,
        &rows,
        &[],
    )
}

/// The cells of the `number`th key: its type, fingerprint and comment.
fn ssh_key_cells(number: usize, key: &str) -> [String; 4] {
    let mut parts = key.split_whitespace();
    let key_type = parts.next().unwrap_or("-").to_string();
    parts.next();
    let comment: Vec<&str> = parts.collect();
    [
        number.to_string(),
        key_type,
        ssh_key_fingerprint(key).unwrap_or_else(|_| "invalid key".to_string()),
        if comment.is_empty() {
            "-".to_string()
        } else {
            comment.join(" ")
        },
    ]
}

/// Render a server status table
#[must_use]
pub fn render_server_status_table(server: &str, status: &str, tls: &str) -> String {
    let row = [server.to_string(), status.to_string(), tls.to_string()];
    render_table(None, SERVER_STATUS_HEADERS, &[row], &[])
}

/// Render where a database backup was written on the server.
#[must_use]
pub fn render_backup_table(backup: &BackupResponse) -> String {
    let rows = [
        ["File on the server".to_string(), backup.path.clone()],
        ["Size (bytes)".to_string(), backup.size_bytes.to_string()],
    ];
    render_table(None, BACKUP_HEADERS, &rows, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_ssh_keys_table() {
        let keys = vec![
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh brad@laptop"
                .to_string(),
            "ssh-rsa broken".to_string(),
        ];
        let table = render_ssh_keys_table("brad", &keys);
        assert!(table.contains("SSH Keys for brad"));
        assert!(table.contains("SHA256:/UpBoWzMiq6/+/wzQIGDT2VxOLzg/LUTfKhc0x+ktEs"));
        assert!(table.contains("brad@laptop"));
        assert!(table.contains("invalid key"));
    }
}
