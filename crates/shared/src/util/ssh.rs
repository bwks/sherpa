use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};
use tracing::instrument;

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use super::file_system::{create_file, expand_path};
use crate::data::{LabInfo, SshKeyAlgorithms, SshPublicKey};
use crate::konst::{
    LAB_FILE_NAME, SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_INDEX_FILE, SHERPA_SSH_INDEX_HEADER,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshConfigEntryStatus {
    Valid,
    Stale,
    Broken,
    Unknown,
}

impl std::fmt::Display for SshConfigEntryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Valid => write!(f, "valid"),
            Self::Stale => write!(f, "stale"),
            Self::Broken => write!(f, "broken"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshConfigInspectionEntry {
    pub line_number: usize,
    pub include_path: String,
    pub lab_id: Option<String>,
    pub status: SshConfigEntryStatus,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshConfigInspectionReport {
    pub index_path: PathBuf,
    pub entries: Vec<SshConfigInspectionEntry>,
    pub server_validation_available: bool,
    pub server_validation_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshConfigCleanReport {
    pub index_path: PathBuf,
    pub kept: Vec<SshConfigInspectionEntry>,
    pub removed: Vec<SshConfigInspectionEntry>,
    pub server_validation_available: bool,
    pub server_validation_message: Option<String>,
}

/// Read an SSH public key file and return a String.
pub fn get_ssh_public_key(path: &str) -> Result<SshPublicKey> {
    let full_path = expand_path(path);
    let file = File::open(&full_path).with_context(|| format!("Error loading file: {path}"))?;
    let reader = BufReader::new(file);

    // Read the first line of the file
    if let Some(line) = reader.lines().next() {
        let key = line?;
        let key_parts: Vec<&str> = key.split_whitespace().collect();

        // Validate that the key contains at least two parts (type and base64 key)
        if key_parts.len() < 2 {
            return Err(anyhow!("Invalid SSH public key format"));
        }
        let algorithm = key_parts[0].parse::<SshKeyAlgorithms>()?;
        let ssh_pub_key = SshPublicKey {
            algorithm,
            key: key_parts[1].to_string(),
            comment: if key_parts.len() == 3 {
                Some(key_parts[2].to_string())
            } else {
                None
            },
        };
        Ok(ssh_pub_key)
    } else {
        Err(anyhow!("Invalid SSH public key file: {full_path}",))
    }
}

/// Convert an SSH public key base64 encoded string to an MD5 Hash
/// expected by Cisco devices.
pub fn pub_ssh_key_to_md5_hash(pub_key_str: &str) -> Result<String> {
    // Decode the base64 encoded key to get binary data
    let binary_key = general_purpose::STANDARD
        .decode(pub_key_str)
        .map_err(|e| anyhow!("Error decoding base64 key: {}", e))?;

    // Compute the MD5 hash of the binary key data
    let md5_hash = md5::compute(binary_key);

    // Format the hash as an uppercase hexadecimal string accepted by cisco devices
    let formatted_hash = format!("{:X}", md5_hash);

    Ok(formatted_hash)
}

/// Convert a base64 encoded SSH public key to an SHA-256 hash
pub fn pub_ssh_key_to_sha256_hash(pub_key_str: &str) -> Result<String> {
    // Decode the base64 encoded key to get binary data
    let binary_key = general_purpose::STANDARD
        .decode(pub_key_str)
        .map_err(|e| anyhow!("Error decoding base64 key: {}", e))?;

    // Compute the SHA-256 hash of the binary key data
    let sha256_hash = Sha256::digest(&binary_key);

    // Format the hash as a lowercase hexadecimal string
    let fingerprint = sha256_hash
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<String>>()
        .join(":");

    Ok(fingerprint)
}

/// Generate an SSH RSA keypair.
pub fn generate_ssh_keypair(directory: &str, keyname: &str, algorithm: Algorithm) -> Result<()> {
    let mut rng = OsRng;

    // Generate a new private key based on the algorithm
    let private_key = match algorithm {
        Algorithm::Rsa { hash } => PrivateKey::random(
            &mut rng,
            Algorithm::Rsa {
                hash: Some(hash.unwrap_or(HashAlg::Sha256)),
            },
        )?,
        Algorithm::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519)?,
        Algorithm::Ecdsa { curve } => PrivateKey::random(&mut rng, Algorithm::Ecdsa { curve })?,
        _ => {
            return Err(anyhow!("Unsupported SSH key algorithm: {:?}", algorithm));
        }
    };

    // Serialize the private key to the OpenSSH format
    let private_key_pem = private_key.to_openssh(LineEnding::LF)?;

    // Extract the public key from the private key
    let public_key = private_key.public_key();
    let public_key_ssh = public_key.to_openssh()?;

    let private_key_path = &format!("{directory}/{keyname}");
    let public_key_path = &format!("{directory}/{keyname}.pub");

    // Create the SSH Public/Private keypair
    create_file(public_key_path, public_key_ssh.to_string())?;
    create_file(private_key_path, private_key_pem.to_string())?;

    // Update permissions of private key file to be read-only.
    #[cfg(unix)]
    {
        let metadata = std::fs::metadata(private_key_path)?;
        let mut perms = metadata.permissions();
        perms.set_mode(0o640);
        std::fs::set_permissions(private_key_path, perms)?;
    }

    Ok(())
}

/// Find SSH public keys in the current user's home directory
///
/// Searches for common SSH key files in ~/.ssh/:
/// - id_rsa.pub
/// - id_ed25519.pub  
/// - id_ecdsa.pub
///
/// Returns a vector of key contents as raw strings (suitable for database storage).
/// Returns an empty vector if no keys are found.
///
pub fn find_user_ssh_keys() -> Vec<String> {
    let mut keys = Vec::new();

    // Get home directory
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return keys, // No home directory found, return empty
    };

    let ssh_dir = home.join(".ssh");
    let ssh_dir = ssh_dir.to_string_lossy();

    // Look for common SSH key files
    for key_type in &["id_rsa.pub", "id_ed25519.pub", "id_ecdsa.pub"] {
        let key_path = format!("{}/{}", ssh_dir, key_type);

        if let Ok(content) = std::fs::read_to_string(&key_path) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                keys.push(trimmed.to_string());
            }
        }
    }

    keys
}

/// Resolve the user's ~/.ssh directory.
fn get_user_ssh_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
    Ok(home.join(".ssh"))
}

fn sherpa_index_path(ssh_dir: &Path) -> PathBuf {
    ssh_dir.join(SHERPA_SSH_INDEX_FILE)
}

fn parse_include_target(line: &str) -> Option<Result<String>> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.first() != Some(&"Include") {
        return Some(Err(anyhow!("Line is not an Include directive")));
    }

    if parts.len() != 2 {
        return Some(Err(anyhow!(
            "Include directive must contain exactly one path"
        )));
    }

    Some(Ok(parts[1].to_string()))
}

fn read_lab_info_for_include(include_path: &Path) -> Result<LabInfo> {
    let lab_dir = include_path
        .parent()
        .ok_or_else(|| anyhow!("Include path has no parent directory"))?;
    let lab_info_path = lab_dir.join(LAB_FILE_NAME);

    let content = std::fs::read_to_string(&lab_info_path)
        .with_context(|| format!("Missing or unreadable {}", lab_info_path.display()))?;

    LabInfo::from_str(&content).with_context(|| format!("Invalid {}", lab_info_path.display()))
}

fn inspect_include_entry(
    line_number: usize,
    include_path: String,
    active_lab_ids: Option<&HashSet<String>>,
    seen_include_paths: &mut HashSet<String>,
) -> SshConfigInspectionEntry {
    if !seen_include_paths.insert(include_path.clone()) {
        return SshConfigInspectionEntry {
            line_number,
            include_path,
            lab_id: None,
            status: SshConfigEntryStatus::Stale,
            reason: "Duplicate Include entry".to_string(),
        };
    }

    let path = PathBuf::from(&include_path);
    if path.file_name().and_then(|name| name.to_str()) != Some(SHERPA_SSH_CONFIG_FILE) {
        return SshConfigInspectionEntry {
            line_number,
            include_path,
            lab_id: None,
            status: SshConfigEntryStatus::Broken,
            reason: format!("Include target is not {SHERPA_SSH_CONFIG_FILE}"),
        };
    }

    if !path.exists() {
        return SshConfigInspectionEntry {
            line_number,
            include_path,
            lab_id: None,
            status: SshConfigEntryStatus::Stale,
            reason: "Included SSH config file does not exist".to_string(),
        };
    }

    let lab_info = match read_lab_info_for_include(&path) {
        Ok(lab_info) => lab_info,
        Err(e) => {
            return SshConfigInspectionEntry {
                line_number,
                include_path,
                lab_id: None,
                status: SshConfigEntryStatus::Broken,
                reason: e.to_string(),
            };
        }
    };

    match active_lab_ids {
        Some(ids) if ids.contains(&lab_info.id) => SshConfigInspectionEntry {
            line_number,
            include_path,
            lab_id: Some(lab_info.id),
            status: SshConfigEntryStatus::Valid,
            reason: "Lab exists on server".to_string(),
        },
        Some(_) => SshConfigInspectionEntry {
            line_number,
            include_path,
            lab_id: Some(lab_info.id),
            status: SshConfigEntryStatus::Stale,
            reason: "Lab ID was not found on server for this user".to_string(),
        },
        None => SshConfigInspectionEntry {
            line_number,
            include_path,
            lab_id: Some(lab_info.id),
            status: SshConfigEntryStatus::Unknown,
            reason: "Local files are valid; server validation was not available".to_string(),
        },
    }
}

fn inspect_lab_ssh_includes_at(
    ssh_dir: &Path,
    active_lab_ids: Option<&HashSet<String>>,
    server_validation_message: Option<String>,
) -> Result<SshConfigInspectionReport> {
    let index_path = sherpa_index_path(ssh_dir);
    let server_validation_available = active_lab_ids.is_some();

    if !index_path.exists() {
        return Ok(SshConfigInspectionReport {
            index_path,
            entries: Vec::new(),
            server_validation_available,
            server_validation_message,
        });
    }

    let content = std::fs::read_to_string(&index_path)
        .with_context(|| format!("Failed to read {}", index_path.display()))?;
    let mut seen_include_paths = HashSet::new();
    let mut entries = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        let line_number = idx + 1;
        match parse_include_target(line) {
            Some(Ok(include_path)) => entries.push(inspect_include_entry(
                line_number,
                include_path,
                active_lab_ids,
                &mut seen_include_paths,
            )),
            Some(Err(e)) => entries.push(SshConfigInspectionEntry {
                line_number,
                include_path: line.trim().to_string(),
                lab_id: None,
                status: SshConfigEntryStatus::Broken,
                reason: e.to_string(),
            }),
            None => {}
        }
    }

    Ok(SshConfigInspectionReport {
        index_path,
        entries,
        server_validation_available,
        server_validation_message,
    })
}

fn clean_stale_lab_ssh_includes_at(
    ssh_dir: &Path,
    active_lab_ids: Option<&HashSet<String>>,
    server_validation_message: Option<String>,
) -> Result<SshConfigCleanReport> {
    let inspection =
        inspect_lab_ssh_includes_at(ssh_dir, active_lab_ids, server_validation_message.clone())?;

    if !inspection.index_path.exists() {
        return Ok(SshConfigCleanReport {
            index_path: inspection.index_path,
            kept: Vec::new(),
            removed: Vec::new(),
            server_validation_available: inspection.server_validation_available,
            server_validation_message: inspection.server_validation_message,
        });
    }

    let removable_lines: HashSet<usize> = inspection
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.status,
                SshConfigEntryStatus::Stale | SshConfigEntryStatus::Broken
            )
        })
        .map(|entry| entry.line_number)
        .collect();

    let content = std::fs::read_to_string(&inspection.index_path)
        .with_context(|| format!("Failed to read {}", inspection.index_path.display()))?;
    let new_content = content
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let line_number = idx + 1;
            if removable_lines.contains(&line_number) {
                None
            } else {
                Some(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    std::fs::write(&inspection.index_path, format!("{}\n", new_content))
        .with_context(|| format!("Failed to write {}", inspection.index_path.display()))?;

    let removed = inspection
        .entries
        .iter()
        .filter(|entry| removable_lines.contains(&entry.line_number))
        .cloned()
        .collect();
    let kept = inspection
        .entries
        .iter()
        .filter(|entry| !removable_lines.contains(&entry.line_number))
        .cloned()
        .collect();

    Ok(SshConfigCleanReport {
        index_path: inspection.index_path,
        kept,
        removed,
        server_validation_available: inspection.server_validation_available,
        server_validation_message: inspection.server_validation_message,
    })
}

/// Ensure ssh_dir/config has a permanent Include line for ssh_dir/sherpa_lab_hosts.
/// Creates the config and sherpa_lab_hosts files if they don't exist.
fn ensure_sherpa_include_in_ssh_config_at(ssh_dir: &std::path::Path) -> Result<std::path::PathBuf> {
    let ssh_config_path = ssh_dir.join("config");
    let sherpa_configs_path = ssh_dir.join(SHERPA_SSH_INDEX_FILE);

    // Ensure ssh_dir exists
    if !ssh_dir.exists() {
        std::fs::create_dir_all(ssh_dir)
            .with_context(|| format!("Failed to create {}", ssh_dir.display()))?;
        #[cfg(unix)]
        {
            std::fs::set_permissions(ssh_dir, PermissionsExt::from_mode(0o700))
                .with_context(|| format!("Failed to set permissions on {}", ssh_dir.display()))?;
        }
    }

    // Ensure sherpa_lab_hosts exists
    if !sherpa_configs_path.exists() {
        std::fs::write(
            &sherpa_configs_path,
            format!("{}\n", SHERPA_SSH_INDEX_HEADER),
        )
        .with_context(|| format!("Failed to create {}", sherpa_configs_path.display()))?;
    }

    let include_line = format!("Include {}", sherpa_configs_path.display());

    // Read existing config or start fresh
    let existing = if ssh_config_path.exists() {
        std::fs::read_to_string(&ssh_config_path)
            .with_context(|| format!("Failed to read {}", ssh_config_path.display()))?
    } else {
        String::new()
    };

    // Already has the Include — done
    if existing.lines().any(|line| line.trim() == include_line) {
        return Ok(sherpa_configs_path);
    }

    // Prepend the Include line (must come before Host blocks)
    let new_content = format!("{}\n{}", include_line, existing);
    std::fs::write(&ssh_config_path, new_content)
        .with_context(|| format!("Failed to write {}", ssh_config_path.display()))?;

    Ok(sherpa_configs_path)
}

/// Add an Include line for a lab's sherpa_ssh_config to the sherpa_lab_hosts index.
fn add_lab_ssh_include_at(ssh_dir: &std::path::Path, lab_ssh_config_path: &str) -> Result<()> {
    let sherpa_configs_path = ensure_sherpa_include_in_ssh_config_at(ssh_dir)?;

    let existing = std::fs::read_to_string(&sherpa_configs_path)
        .with_context(|| format!("Failed to read {}", sherpa_configs_path.display()))?;

    let include_line = format!("Include {}", lab_ssh_config_path);

    // Already present — idempotent
    if existing.lines().any(|line| line.trim() == include_line) {
        return Ok(());
    }

    let new_content = format!("{}{}\n", existing, include_line);
    std::fs::write(&sherpa_configs_path, new_content)
        .with_context(|| format!("Failed to write {}", sherpa_configs_path.display()))?;

    Ok(())
}

/// Remove an Include line for a lab's sherpa_ssh_config from the sherpa_lab_hosts index.
fn remove_lab_ssh_include_at(ssh_dir: &std::path::Path, lab_ssh_config_path: &str) -> Result<()> {
    let sherpa_configs_path = ssh_dir.join(SHERPA_SSH_INDEX_FILE);

    if !sherpa_configs_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&sherpa_configs_path)
        .with_context(|| format!("Failed to read {}", sherpa_configs_path.display()))?;

    let include_line = format!("Include {}", lab_ssh_config_path);

    if !content.lines().any(|line| line.trim() == include_line) {
        return Ok(());
    }

    let new_content: String = content
        .lines()
        .filter(|line| line.trim() != include_line)
        .collect::<Vec<_>>()
        .join("\n");

    std::fs::write(&sherpa_configs_path, format!("{}\n", new_content))
        .with_context(|| format!("Failed to write {}", sherpa_configs_path.display()))?;

    Ok(())
}

/// Add an Include line for a lab's sherpa_ssh_config to ~/.ssh/sherpa_lab_hosts.
/// Also ensures the permanent Include in ~/.ssh/config exists.
#[instrument(fields(%lab_ssh_config_path))]
pub fn add_lab_ssh_include(lab_ssh_config_path: &str) -> Result<()> {
    let ssh_dir = get_user_ssh_dir()?;
    add_lab_ssh_include_at(&ssh_dir, lab_ssh_config_path)
}

/// Remove an Include line for a lab's sherpa_ssh_config from ~/.ssh/sherpa_lab_hosts.
#[instrument(fields(%lab_ssh_config_path))]
pub fn remove_lab_ssh_include(lab_ssh_config_path: &str) -> Result<()> {
    let ssh_dir = get_user_ssh_dir()?;
    remove_lab_ssh_include_at(&ssh_dir, lab_ssh_config_path)
}

/// Inspect ~/.ssh/sherpa_lab_hosts for stale or broken Sherpa lab Include entries.
#[instrument(skip(active_lab_ids))]
pub fn inspect_lab_ssh_includes(
    active_lab_ids: Option<&HashSet<String>>,
    server_validation_message: Option<String>,
) -> Result<SshConfigInspectionReport> {
    let ssh_dir = get_user_ssh_dir()?;
    inspect_lab_ssh_includes_at(&ssh_dir, active_lab_ids, server_validation_message)
}

/// Remove stale or broken Sherpa lab Include entries from ~/.ssh/sherpa_lab_hosts.
#[instrument(skip(active_lab_ids))]
pub fn clean_stale_lab_ssh_includes(
    active_lab_ids: Option<&HashSet<String>>,
    server_validation_message: Option<String>,
) -> Result<SshConfigCleanReport> {
    let ssh_dir = get_user_ssh_dir()?;
    clean_stale_lab_ssh_includes_at(&ssh_dir, active_lab_ids, server_validation_message)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;

    // ========================================================================
    // SSH Include management tests
    // ========================================================================

    fn read_file(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn test_add_creates_sherpa_lab_hosts_if_missing() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let index_path = ssh_dir.join(SHERPA_SSH_INDEX_FILE);
        assert!(index_path.exists());
        let content = read_file(&index_path);
        assert!(content.contains("Include /tmp/lab1/sherpa_ssh_config"));
    }

    #[test]
    fn test_add_creates_ssh_dir_if_missing() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        // Do NOT create ssh_dir — let the function do it

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        assert!(ssh_dir.exists());
        assert!(ssh_dir.join(SHERPA_SSH_INDEX_FILE).exists());
    }

    #[test]
    fn test_add_include_line_to_sherpa_lab_hosts() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/home/user/lab1/sherpa_ssh_config").unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(content.contains(SHERPA_SSH_INDEX_HEADER));
        assert!(content.contains("Include /home/user/lab1/sherpa_ssh_config"));
    }

    #[test]
    fn test_add_prepends_include_to_ssh_config() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        // Pre-populate ~/.ssh/config with existing content
        let ssh_config = ssh_dir.join("config");
        std::fs::write(&ssh_config, "Host myserver\n    HostName 1.2.3.4\n").unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let content = read_file(&ssh_config);
        let include_line = format!("Include {}", ssh_dir.join(SHERPA_SSH_INDEX_FILE).display());
        assert!(content.starts_with(&include_line));
        assert!(content.contains("Host myserver"));
    }

    #[test]
    fn test_add_idempotent_ssh_config_include() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let content = read_file(&ssh_dir.join("config"));
        let include_line = format!("Include {}", ssh_dir.join(SHERPA_SSH_INDEX_FILE).display());
        let count = content.matches(&include_line).count();
        assert_eq!(count, 1, "Include in ssh/config should appear exactly once");
    }

    #[test]
    fn test_add_idempotent_lab_entry() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        let count = content
            .matches("Include /tmp/lab1/sherpa_ssh_config")
            .count();
        assert_eq!(count, 1, "Lab include should appear exactly once");
    }

    #[test]
    fn test_add_preserves_existing_ssh_config_content() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        let existing = "Host myserver\n    HostName 1.2.3.4\n    User admin\n";
        std::fs::write(ssh_dir.join("config"), existing).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let content = read_file(&ssh_dir.join("config"));
        assert!(content.contains("Host myserver"));
        assert!(content.contains("HostName 1.2.3.4"));
        assert!(content.contains("User admin"));
    }

    #[test]
    fn test_add_multiple_labs() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/home/user/lab1/sherpa_ssh_config").unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/home/user/lab2/sherpa_ssh_config").unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(content.contains("Include /home/user/lab1/sherpa_ssh_config"));
        assert!(content.contains("Include /home/user/lab2/sherpa_ssh_config"));
    }

    #[test]
    fn test_remove_correct_include_line() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab2/sherpa_ssh_config").unwrap();

        remove_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(!content.contains("Include /tmp/lab1/sherpa_ssh_config"));
        assert!(content.contains("Include /tmp/lab2/sherpa_ssh_config"));
    }

    #[test]
    fn test_remove_leaves_other_labs_intact() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab2/sherpa_ssh_config").unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab3/sherpa_ssh_config").unwrap();

        remove_lab_ssh_include_at(&ssh_dir, "/tmp/lab2/sherpa_ssh_config").unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(content.contains("Include /tmp/lab1/sherpa_ssh_config"));
        assert!(!content.contains("Include /tmp/lab2/sherpa_ssh_config"));
        assert!(content.contains("Include /tmp/lab3/sherpa_ssh_config"));
    }

    #[test]
    fn test_remove_no_error_when_index_missing() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        // No sherpa_lab_hosts file exists

        let result = remove_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config");
        assert!(result.is_ok());
    }

    #[test]
    fn test_remove_no_error_when_line_not_present() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let result = remove_lab_ssh_include_at(&ssh_dir, "/tmp/nonexistent/sherpa_ssh_config");
        assert!(result.is_ok());

        // Original entry still intact
        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(content.contains("Include /tmp/lab1/sherpa_ssh_config"));
    }

    #[test]
    fn test_remove_does_not_modify_ssh_config() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let config_before = read_file(&ssh_dir.join("config"));

        remove_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let config_after = read_file(&ssh_dir.join("config"));
        assert_eq!(
            config_before, config_after,
            "~/.ssh/config should not be modified by remove"
        );
    }

    fn valid_lab_info_toml(lab_id: &str) -> String {
        format!(
            r#"id = "{lab_id}"
name = "test-lab"
user = "test-user"
ipv4_network = "172.31.1.0/24"
ipv4_gateway = "172.31.1.1"
ipv4_router = "172.31.1.254"
loopback_network = "127.127.1.0/24"
"#
        )
    }

    fn create_lab_files(base: &Path, lab_dir_name: &str, lab_id: &str) -> String {
        let lab_dir = base.join(lab_dir_name);
        std::fs::create_dir_all(&lab_dir).unwrap();
        let ssh_config_path = lab_dir.join(SHERPA_SSH_CONFIG_FILE);
        std::fs::write(&ssh_config_path, "Host test\n").unwrap();
        std::fs::write(lab_dir.join(LAB_FILE_NAME), valid_lab_info_toml(lab_id)).unwrap();
        ssh_config_path.to_string_lossy().to_string()
    }

    #[test]
    fn test_inspect_reports_valid_when_lab_is_active() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let include_path = create_lab_files(tmp.path(), "lab1", "lab-active");
        add_lab_ssh_include_at(&ssh_dir, &include_path).unwrap();

        let mut active = HashSet::new();
        active.insert("lab-active".to_string());
        let report = inspect_lab_ssh_includes_at(&ssh_dir, Some(&active), None).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].status, SshConfigEntryStatus::Valid);
        assert_eq!(report.entries[0].lab_id.as_deref(), Some("lab-active"));
    }

    #[test]
    fn test_inspect_reports_stale_when_lab_missing_from_server() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let include_path = create_lab_files(tmp.path(), "lab1", "deleted-lab");
        add_lab_ssh_include_at(&ssh_dir, &include_path).unwrap();

        let active = HashSet::new();
        let report = inspect_lab_ssh_includes_at(&ssh_dir, Some(&active), None).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].status, SshConfigEntryStatus::Stale);
        assert_eq!(report.entries[0].lab_id.as_deref(), Some("deleted-lab"));
    }

    #[test]
    fn test_inspect_reports_stale_when_config_missing() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/tmp/missing/sherpa_ssh_config").unwrap();

        let report = inspect_lab_ssh_includes_at(&ssh_dir, None, None).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].status, SshConfigEntryStatus::Stale);
    }

    #[test]
    fn test_inspect_reports_broken_when_lab_info_invalid() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let lab_dir = tmp.path().join("lab1");
        std::fs::create_dir_all(&lab_dir).unwrap();
        let ssh_config_path = lab_dir.join(SHERPA_SSH_CONFIG_FILE);
        std::fs::write(&ssh_config_path, "Host test\n").unwrap();
        std::fs::write(lab_dir.join(LAB_FILE_NAME), "not = valid = toml").unwrap();
        add_lab_ssh_include_at(&ssh_dir, &ssh_config_path.to_string_lossy()).unwrap();

        let report = inspect_lab_ssh_includes_at(&ssh_dir, None, None).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].status, SshConfigEntryStatus::Broken);
    }

    #[test]
    fn test_clean_removes_stale_and_keeps_valid() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let valid_include = create_lab_files(tmp.path(), "lab1", "active-lab");
        let stale_include = create_lab_files(tmp.path(), "lab2", "deleted-lab");
        add_lab_ssh_include_at(&ssh_dir, &valid_include).unwrap();
        add_lab_ssh_include_at(&ssh_dir, &stale_include).unwrap();

        let mut active = HashSet::new();
        active.insert("active-lab".to_string());
        let report = clean_stale_lab_ssh_includes_at(&ssh_dir, Some(&active), None).unwrap();

        assert_eq!(report.kept.len(), 1);
        assert_eq!(report.removed.len(), 1);
        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(content.contains(&valid_include));
        assert!(!content.contains(&stale_include));
        assert!(content.contains(SHERPA_SSH_INDEX_HEADER));
    }

    #[test]
    fn test_clean_removes_duplicate_include() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let include_path = create_lab_files(tmp.path(), "lab1", "active-lab");
        ensure_sherpa_include_in_ssh_config_at(&ssh_dir).unwrap();
        std::fs::write(
            ssh_dir.join(SHERPA_SSH_INDEX_FILE),
            format!(
                "{}\nInclude {}\nInclude {}\n",
                SHERPA_SSH_INDEX_HEADER, include_path, include_path
            ),
        )
        .unwrap();

        let mut active = HashSet::new();
        active.insert("active-lab".to_string());
        let report = clean_stale_lab_ssh_includes_at(&ssh_dir, Some(&active), None).unwrap();

        assert_eq!(report.kept.len(), 1);
        assert_eq!(report.removed.len(), 1);
        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert_eq!(content.matches(&include_path).count(), 1);
    }

    #[test]
    fn test_remove_uses_exact_line_matching() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        ensure_sherpa_include_in_ssh_config_at(&ssh_dir).unwrap();
        std::fs::write(
            ssh_dir.join(SHERPA_SSH_INDEX_FILE),
            format!(
                "{}\nInclude /tmp/lab1/sherpa_ssh_config-extra\nInclude /tmp/lab1/sherpa_ssh_config\n",
                SHERPA_SSH_INDEX_HEADER
            ),
        )
        .unwrap();

        remove_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/sherpa_ssh_config").unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(content.contains("Include /tmp/lab1/sherpa_ssh_config-extra"));
        assert!(
            !content
                .lines()
                .any(|line| line.trim() == "Include /tmp/lab1/sherpa_ssh_config")
        );
    }

    // ========================================================================
    // SSH key hash tests
    // ========================================================================

    #[test]
    fn test_pub_ssh_key_to_md5_hash() {
        let test_cases = vec![
            (
                "AAAAB3NzaC1yc2EAAAABIwAAAQEA6NF8iallvQVp22WDkTkyrtvp9eWW6A8YVr+kz4TjGYe7gHzIw+niNltGEFHzD8+v1I2YJ6oXevct1YeS0o9HZyN1Q9qgCgzUFtdOKLv6IedplqoPkcmF0aYet2PkEDo3MlTBckFXPITAMzF8dJSIFo9D8HfdOV0IAdx4O7PtixWKn5y2hMNG0zQPyUecp4pzC6kivAIhyfHilFR61RGL+GPXQ2MWZWFYbAGjyiYJnAmCP3NOTd0jMZEnDkbUvxhMmBYSdETk1rRgm+R4LOzFUGaHqHDLKLX+FIPKcF96hrucXzcWyLbIbEgE98OHlnVYCzRdK8jlqm8tehUc9c9WhQ==",
                "DD3BB82E850406E9ABFFA80AC0046ED6",
            ),
            (
                "AAAAC3NzaC1lZDI1NTE5AAAAIPxpcrq+EIzKyYav9c/h3BRAHcv4M1fzSDY7OhGDwFf+",
                "51A61C4047F60237A80D2D4B02ED7885",
            ),
            // Add more test cases as needed
        ];

        for (input, expected) in test_cases {
            assert_eq!(pub_ssh_key_to_md5_hash(input).unwrap(), expected);
        }
    }

    #[test]
    fn test_pub_ssh_key_to_sha256_hash() {
        let test_cases = vec![
            (
                "AAAAB3NzaC1yc2EAAAABIwAAAQEA6NF8iallvQVp22WDkTkyrtvp9eWW6A8YVr+kz4TjGYe7gHzIw+niNltGEFHzD8+v1I2YJ6oXevct1YeS0o9HZyN1Q9qgCgzUFtdOKLv6IedplqoPkcmF0aYet2PkEDo3MlTBckFXPITAMzF8dJSIFo9D8HfdOV0IAdx4O7PtixWKn5y2hMNG0zQPyUecp4pzC6kivAIhyfHilFR61RGL+GPXQ2MWZWFYbAGjyiYJnAmCP3NOTd0jMZEnDkbUvxhMmBYSdETk1rRgm+R4LOzFUGaHqHDLKLX+FIPKcF96hrucXzcWyLbIbEgE98OHlnVYCzRdK8jlqm8tehUc9c9WhQ==",
                "d4:ce:11:ce:13:32:5a:e1:52:ff:ce:ae:3d:8f:dc:7b:6a:6b:87:f7:55:4c:75:bb:88:3d:91:86:9a:ae:39:90",
            ),
            (
                "AAAAC3NzaC1lZDI1NTE5AAAAIPxpcrq+EIzKyYav9c/h3BRAHcv4M1fzSDY7OhGDwFf+",
                "8c:7f:4d:2a:44:e3:fe:56:80:f2:5d:b5:4e:8a:f1:7a:71:a6:4d:18:2d:ab:3d:c9:ab:26:10:69:15:cd:8b:61",
            ),
        ];

        for (input, expected) in test_cases {
            assert_eq!(pub_ssh_key_to_sha256_hash(input).unwrap(), expected);
        }
    }

    #[test]
    fn test_pub_ssh_key_to_sha256_hash_invalid_input() {
        let invalid_key = "not a valid ssh key";
        assert!(pub_ssh_key_to_sha256_hash(invalid_key).is_err());
    }

    #[test]
    fn test_pub_ssh_key_to_md5_hash_invalid_input() {
        let invalid_key = "not a valid ssh key";
        assert!(pub_ssh_key_to_md5_hash(invalid_key).is_err());
    }
}
