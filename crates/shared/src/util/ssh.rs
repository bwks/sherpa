use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose};
use sha2::{Digest, Sha256};
use ssh_key::{HashAlg, PublicKey};
use tempfile::NamedTempFile;
use tracing::instrument;

use std::collections::{BTreeSet, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use super::file_system::expand_path;
use super::lab_folder::LabFolder;
use crate::data::{LabInfo, SshKeyAlgorithms, SshPublicKey};
use crate::konst::{SHERPA_SSH_INDEX_FILE, SHERPA_SSH_INDEX_HEADER};

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

/// The lock file that serialises Sherpa's changes to the SSH config files.
const SSH_LOCK_FILE: &str = "sherpa_lab_hosts.lock";

/// Read an SSH public key file and return a String.
///
/// # Errors
///
/// Fails when the file cannot be read or is not an OpenSSH public key.
pub fn get_ssh_public_key(path: &str) -> Result<SshPublicKey> {
    let full_path = expand_path(path);
    let file = File::open(&full_path).with_context(|| format!("Error loading file: {path}"))?;
    let line = BufReader::new(file)
        .lines()
        .next()
        .with_context(|| format!("Invalid SSH public key file: {full_path}"))??;
    let public_key = PublicKey::from_openssh(line.trim())
        .with_context(|| format!("Invalid SSH public key in {full_path}"))?;
    let algorithm = public_key
        .algorithm()
        .as_str()
        .parse::<SshKeyAlgorithms>()?;
    let key = general_purpose::STANDARD.encode(
        public_key
            .to_bytes()
            .context("Failed to encode the SSH public key")?,
    );
    let comment = Some(public_key.comment().to_owned()).filter(|comment| !comment.is_empty());
    Ok(SshPublicKey {
        algorithm,
        key,
        comment,
    })
}

/// Convert an SSH public key base64 encoded string to an MD5 Hash
/// expected by Cisco devices.
///
/// # Errors
///
/// Fails when the key data is not base64.
pub fn pub_ssh_key_to_md5_hash(pub_key_str: &str) -> Result<String> {
    // Decode the base64 encoded key to get binary data
    let binary_key = general_purpose::STANDARD
        .decode(pub_key_str)
        .map_err(|e| anyhow!("Error decoding base64 key: {e}"))?;

    // Compute the MD5 hash of the binary key data
    let md5_hash = md5::compute(binary_key);

    // Format the hash as an uppercase hexadecimal string accepted by cisco devices
    let formatted_hash = format!("{md5_hash:X}");

    Ok(formatted_hash)
}

/// Convert a base64 encoded SSH public key to an SHA-256 hash
///
/// # Errors
///
/// Fails when the key data is not base64.
pub fn pub_ssh_key_to_sha256_hash(pub_key_str: &str) -> Result<String> {
    // Decode the base64 encoded key to get binary data
    let binary_key = general_purpose::STANDARD
        .decode(pub_key_str)
        .map_err(|e| anyhow!("Error decoding base64 key: {e}"))?;

    // Compute the SHA-256 hash of the binary key data
    let sha256_hash = Sha256::digest(&binary_key);

    // Format the hash as a lowercase hexadecimal string
    let fingerprint = sha256_hash
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<String>>()
        .join(":");

    Ok(fingerprint)
}

/// The SHA256 fingerprint of an OpenSSH public key, as `ssh-keygen -l` prints it
/// (`SHA256:...`).
///
/// # Errors
///
/// Fails when `public_key` is not an OpenSSH public key.
pub fn ssh_key_fingerprint(public_key: &str) -> Result<String> {
    let key = PublicKey::from_openssh(public_key.trim()).context("Invalid SSH public key")?;
    Ok(key.fingerprint(HashAlg::Sha256).to_string())
}

/// The user's `~/.ssh` folder.
///
/// # Errors
///
/// Fails when the home directory is unknown.
pub fn get_user_ssh_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
    Ok(home.join(".ssh"))
}

fn sherpa_index_path(ssh_dir: &Path) -> PathBuf {
    ssh_dir.join(SHERPA_SSH_INDEX_FILE)
}

/// The path of an `Include` line, unquoting a path written in double quotes.
fn parse_include_target(line: &str) -> Option<Result<String>> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    let Some(target) = trimmed
        .strip_prefix("Include")
        .filter(|rest| rest.starts_with(char::is_whitespace))
    else {
        return Some(Err(anyhow!("Line is not an Include directive")));
    };
    let target = target.trim();

    let target = match target
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
    {
        Some(unquoted) if !unquoted.contains('"') => unquoted,
        Some(_) => {
            return Some(Err(anyhow!(
                "Include directive must contain exactly one path"
            )));
        }
        None if target.contains(char::is_whitespace) || target.contains('"') => {
            return Some(Err(anyhow!(
                "Include directive must contain exactly one path"
            )));
        }
        None => target,
    };

    Some(Ok(target.to_string()))
}

/// The `Include` line for a path, quoted when the path holds whitespace, as ssh reads it.
fn include_directive(path: &str) -> Result<String> {
    if path.contains('"') {
        bail!("ssh cannot include a path with a double quote: {path}");
    }
    if path.contains(char::is_whitespace) {
        Ok(format!(r#"Include "{path}""#))
    } else {
        Ok(format!("Include {path}"))
    }
}

/// Whether `content` has an `Include` line for `path`, quoted or not.
fn has_include(content: &str, path: &str) -> bool {
    content
        .lines()
        .any(|line| matches!(parse_include_target(line), Some(Ok(target)) if target == path))
}

/// Replace a file's contents in one step: write a temporary file beside it with the old file's
/// mode, then rename it over the old one, so a crash or a full disk never leaves it cut short.
/// A symlinked file (a dotfiles manager's) is followed, so the link stays in place.
fn replace_file(path: &Path, contents: &str) -> Result<()> {
    let target = match std::fs::canonicalize(path) {
        Ok(real) => real,
        Err(error) if error.kind() == ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to resolve {}", path.display()));
        }
    };
    let folder = target
        .parent()
        .with_context(|| format!("{} has no parent folder", target.display()))?;
    let mut temporary = NamedTempFile::new_in(folder)
        .with_context(|| format!("Failed to create a file in {}", folder.display()))?;
    if let Ok(metadata) = std::fs::metadata(&target) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .with_context(|| format!("Failed to copy the mode of {}", target.display()))?;
    }
    temporary
        .write_all(contents.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .with_context(|| format!("Failed to write {}", target.display()))?;
    temporary
        .persist(&target)
        .with_context(|| format!("Failed to replace {}", target.display()))?;
    Ok(())
}

/// Hold the lock that serialises Sherpa's changes to the SSH config files; it is released when
/// the returned file is dropped.
fn lock_ssh_dir(ssh_dir: &Path) -> Result<File> {
    let path = ssh_dir.join(SSH_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("Failed to open {}", path.display()))?;
    file.lock()
        .with_context(|| format!("Failed to lock {}", path.display()))?;
    Ok(file)
}

/// Create the SSH folder, owner only, if it does not exist.
fn create_ssh_dir(ssh_dir: &Path) -> Result<()> {
    if !ssh_dir.exists() {
        std::fs::create_dir_all(ssh_dir)
            .with_context(|| format!("Failed to create {}", ssh_dir.display()))?;
        #[cfg(unix)]
        {
            std::fs::set_permissions(ssh_dir, PermissionsExt::from_mode(0o700))
                .with_context(|| format!("Failed to set permissions on {}", ssh_dir.display()))?;
        }
    }
    Ok(())
}

fn read_lab_info_for_include(lab_folder: &LabFolder) -> Result<LabInfo> {
    let lab_info_path = lab_folder.lab_info();

    let content = std::fs::read_to_string(&lab_info_path)
        .with_context(|| format!("Missing or unreadable {}", lab_info_path.display()))?;

    LabInfo::from_str(&content).with_context(|| format!("Invalid {}", lab_info_path.display()))
}

fn inspect_include_entry(
    line_number: usize,
    include_path: String,
    active_lab_ids: Option<&BTreeSet<String>>,
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
    let Some(lab_folder) = LabFolder::of_ssh_config(&path) else {
        return SshConfigInspectionEntry {
            line_number,
            include_path,
            lab_id: None,
            status: SshConfigEntryStatus::Broken,
            reason: "Include target is not a lab's .sherpa/ssh/config".to_string(),
        };
    };

    if !path.exists() {
        return SshConfigInspectionEntry {
            line_number,
            include_path,
            lab_id: None,
            status: SshConfigEntryStatus::Stale,
            reason: "Included SSH config file does not exist".to_string(),
        };
    }

    let lab_info = match read_lab_info_for_include(&lab_folder) {
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
    active_lab_ids: Option<&BTreeSet<String>>,
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
    active_lab_ids: Option<&BTreeSet<String>>,
    server_validation_message: Option<String>,
) -> Result<SshConfigCleanReport> {
    let _lock = if ssh_dir.exists() {
        Some(lock_ssh_dir(ssh_dir)?)
    } else {
        None
    };
    let inspection =
        inspect_lab_ssh_includes_at(ssh_dir, active_lab_ids, server_validation_message)?;

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

    replace_file(&inspection.index_path, &format!("{new_content}\n"))?;

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

/// Ensure `ssh_dir/config` has a permanent Include line for `ssh_dir/sherpa_lab_hosts`.
/// Creates the config and `sherpa_lab_hosts` files if they don't exist. The caller holds the
/// SSH folder lock.
fn ensure_sherpa_include_in_ssh_config_at(ssh_dir: &Path) -> Result<PathBuf> {
    let ssh_config_path = ssh_dir.join("config");
    let sherpa_configs_path = ssh_dir.join(SHERPA_SSH_INDEX_FILE);

    // Ensure sherpa_lab_hosts exists
    if !sherpa_configs_path.exists() {
        replace_file(
            &sherpa_configs_path,
            &format!("{SHERPA_SSH_INDEX_HEADER}\n"),
        )?;
    }

    let index_path = sherpa_configs_path.display().to_string();

    // Read existing config or start fresh
    let existing = if ssh_config_path.exists() {
        std::fs::read_to_string(&ssh_config_path)
            .with_context(|| format!("Failed to read {}", ssh_config_path.display()))?
    } else {
        String::new()
    };

    // Already has the Include — done
    if has_include(&existing, &index_path) {
        return Ok(sherpa_configs_path);
    }

    // Prepend the Include line (must come before Host blocks)
    let include_line = include_directive(&index_path)?;
    replace_file(&ssh_config_path, &format!("{include_line}\n{existing}"))?;

    Ok(sherpa_configs_path)
}

/// Add an Include line for a lab's `.sherpa/ssh/config` to the `sherpa_lab_hosts` index.
fn add_lab_ssh_include_at(ssh_dir: &Path, lab_ssh_config_path: &str) -> Result<()> {
    let include_line = include_directive(lab_ssh_config_path)?;
    create_ssh_dir(ssh_dir)?;
    let _lock = lock_ssh_dir(ssh_dir)?;
    let sherpa_configs_path = ensure_sherpa_include_in_ssh_config_at(ssh_dir)?;

    let existing = std::fs::read_to_string(&sherpa_configs_path)
        .with_context(|| format!("Failed to read {}", sherpa_configs_path.display()))?;

    // Already present — idempotent
    if has_include(&existing, lab_ssh_config_path) {
        return Ok(());
    }

    replace_file(&sherpa_configs_path, &format!("{existing}{include_line}\n"))?;

    Ok(())
}

/// Remove an Include line for a lab's SSH config from the `sherpa_lab_hosts` index in `ssh_dir`.
///
/// # Errors
///
/// Fails when the SSH config files cannot be locked, read or replaced.
pub fn remove_lab_ssh_include_at(ssh_dir: &Path, lab_ssh_config_path: &str) -> Result<()> {
    let sherpa_configs_path = ssh_dir.join(SHERPA_SSH_INDEX_FILE);

    if !sherpa_configs_path.exists() {
        return Ok(());
    }

    let _lock = lock_ssh_dir(ssh_dir)?;
    let content = std::fs::read_to_string(&sherpa_configs_path)
        .with_context(|| format!("Failed to read {}", sherpa_configs_path.display()))?;

    if !has_include(&content, lab_ssh_config_path) {
        return Ok(());
    }

    let new_content = content
        .lines()
        .filter(|line| !has_include(line, lab_ssh_config_path))
        .collect::<Vec<_>>()
        .join("\n");

    replace_file(&sherpa_configs_path, &format!("{new_content}\n"))?;

    Ok(())
}

/// Point the `Include` for the lab SSH config `old_path` in `ssh_dir/sherpa_lab_hosts` at
/// `new_path` instead, as when a lab's files move. Nothing changes when no line includes
/// `old_path`; a line for `old_path` is dropped when `new_path` is already included. Returns
/// whether the index changed.
///
/// # Errors
///
/// Fails when `new_path` cannot be written as an `Include`, or the index cannot be locked, read
/// or replaced.
#[instrument(level = "debug", fields(ssh_dir = %ssh_dir.display(), %old_path, %new_path))]
pub fn replace_lab_ssh_include(ssh_dir: &Path, old_path: &str, new_path: &str) -> Result<bool> {
    let index_path = sherpa_index_path(ssh_dir);
    if !index_path.exists() {
        return Ok(false);
    }
    let include_line = include_directive(new_path)?;
    let _lock = lock_ssh_dir(ssh_dir)?;
    let content = std::fs::read_to_string(&index_path)
        .with_context(|| format!("Failed to read {}", index_path.display()))?;
    if !has_include(&content, old_path) {
        return Ok(false);
    }
    let mut new_included = has_include(&content, new_path);
    let mut lines = Vec::new();
    for line in content.lines() {
        if !has_include(line, old_path) {
            lines.push(line.to_string());
        } else if !new_included {
            lines.push(include_line.clone());
            new_included = true;
        }
    }
    replace_file(&index_path, &format!("{}\n", lines.join("\n")))?;
    Ok(true)
}

/// Add an Include line for a lab's `.sherpa/ssh/config` to `~/.ssh/sherpa_lab_hosts`.
/// Also ensures the permanent Include in ~/.ssh/config exists.
///
/// # Errors
///
/// Fails when the path cannot be written as an `Include`, or the SSH config files cannot be locked, read or replaced.
#[instrument(fields(%lab_ssh_config_path))]
pub fn add_lab_ssh_include(lab_ssh_config_path: &str) -> Result<()> {
    let ssh_dir = get_user_ssh_dir()?;
    add_lab_ssh_include_at(&ssh_dir, lab_ssh_config_path)
}

/// Inspect `~/.ssh/sherpa_lab_hosts` for stale or broken Sherpa lab Include entries.
///
/// # Errors
///
/// Fails when the home directory is unknown or the index cannot be read.
#[instrument(skip(active_lab_ids))]
pub fn inspect_lab_ssh_includes(
    active_lab_ids: Option<&BTreeSet<String>>,
    server_validation_message: Option<String>,
) -> Result<SshConfigInspectionReport> {
    let ssh_dir = get_user_ssh_dir()?;
    inspect_lab_ssh_includes_at(&ssh_dir, active_lab_ids, server_validation_message)
}

/// Remove stale or broken Sherpa lab Include entries from `~/.ssh/sherpa_lab_hosts`.
///
/// # Errors
///
/// Fails when the home directory is unknown, or the index cannot be locked, read or replaced.
#[instrument(skip(active_lab_ids))]
pub fn clean_stale_lab_ssh_includes(
    active_lab_ids: Option<&BTreeSet<String>>,
    server_validation_message: Option<String>,
) -> Result<SshConfigCleanReport> {
    let ssh_dir = get_user_ssh_dir()?;
    clean_stale_lab_ssh_includes_at(&ssh_dir, active_lab_ids, server_validation_message)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs::Permissions;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
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
    fn ssh_config_include_002_the_first_lab_makes_ssh_and_sherpa_lab_hosts() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");

        add_lab_ssh_include_at(&ssh_dir, "/home/user/lab1/.sherpa/ssh/config").unwrap();

        assert!(ssh_dir.is_dir());
        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(content.contains(SHERPA_SSH_INDEX_HEADER), "{content}");
        assert!(
            content.contains("Include /home/user/lab1/.sherpa/ssh/config"),
            "{content}"
        );
    }

    #[test]
    fn ssh_config_include_003_the_include_goes_above_the_user_s_hosts() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        // Pre-populate ~/.ssh/config with existing content
        let ssh_config = ssh_dir.join("config");
        std::fs::write(&ssh_config, "Host myserver\n    HostName 1.2.3.4\n").unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/.sherpa/ssh/config").unwrap();

        let content = read_file(&ssh_config);
        let include_line = format!("Include {}", ssh_dir.join(SHERPA_SSH_INDEX_FILE).display());
        assert!(content.starts_with(&include_line));
        assert!(content.contains("Host myserver"));
    }

    #[test]
    fn ssh_config_include_004_a_lab_listed_twice_is_in_the_file_once() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/.sherpa/ssh/config").unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/.sherpa/ssh/config").unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        let count = content
            .matches("Include /tmp/lab1/.sherpa/ssh/config")
            .count();
        assert_eq!(count, 1, "Lab include should appear exactly once");
    }

    #[test]
    fn ssh_config_include_009_removing_a_lab_removes_its_line_alone() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        for lab in [
            "/tmp/lab1/.sherpa/ssh/config",
            "/tmp/lab2/.sherpa/ssh/config-extra",
            "/tmp/lab2/.sherpa/ssh/config",
            "/tmp/lab3/.sherpa/ssh/config",
        ] {
            add_lab_ssh_include_at(&ssh_dir, lab).unwrap();
        }
        let config_before = read_file(&ssh_dir.join("config"));

        remove_lab_ssh_include_at(&ssh_dir, "/tmp/lab2/.sherpa/ssh/config").unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        let includes: Vec<&str> = content
            .lines()
            .filter(|line| line.starts_with("Include "))
            .collect();
        assert_eq!(
            includes,
            [
                "Include /tmp/lab1/.sherpa/ssh/config",
                "Include /tmp/lab2/.sherpa/ssh/config-extra",
                "Include /tmp/lab3/.sherpa/ssh/config",
            ]
        );
        assert_eq!(read_file(&ssh_dir.join("config")), config_before);
    }

    #[test]
    fn ssh_config_include_010_removing_a_lab_that_is_not_listed_changes_nothing() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();

        remove_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/.sherpa/ssh/config").unwrap();
        assert!(!ssh_dir.join(SHERPA_SSH_INDEX_FILE).exists());

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/.sherpa/ssh/config").unwrap();
        let before = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        remove_lab_ssh_include_at(&ssh_dir, "/tmp/nonexistent/.sherpa/ssh/config").unwrap();
        assert_eq!(read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE)), before);
    }

    fn valid_lab_info_toml(lab_id: &str) -> String {
        format!(
            r#"id = "{lab_id}"
name = "test-lab"
user = "test-user"
ipv4_network = "172.31.1.0/24"
ipv4_gateway = "172.31.1.1"
ipv4_router = "172.31.1.254"
"#
        )
    }

    fn create_lab_files(base: &Path, lab_dir_name: &str, lab_id: &str) -> String {
        let lab_folder = LabFolder::new(&base.join(lab_dir_name));
        std::fs::create_dir_all(lab_folder.ssh_dir()).unwrap();
        let ssh_config_path = lab_folder.ssh_config();
        std::fs::write(&ssh_config_path, "Host test\n").unwrap();
        std::fs::write(lab_folder.lab_info(), valid_lab_info_toml(lab_id)).unwrap();
        ssh_config_path.to_string_lossy().to_string()
    }

    #[test]
    fn ssh_config_include_005_a_lab_folder_with_a_space_is_included_quoted() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        let include_path = create_lab_files(tmp.path(), "my labs", "lab-space");
        add_lab_ssh_include_at(&ssh_dir, &include_path).unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(
            content.contains(&format!(r#"Include "{include_path}""#)),
            "{content}"
        );

        let active = BTreeSet::from(["lab-space".to_string()]);
        let report = clean_stale_lab_ssh_includes_at(&ssh_dir, Some(&active), None).unwrap();
        assert!(report.removed.is_empty(), "{:?}", report.removed);
        assert_eq!(report.kept[0].status, SshConfigEntryStatus::Valid);

        remove_lab_ssh_include_at(&ssh_dir, &include_path).unwrap();
        assert!(!read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE)).contains("my labs"));
    }

    #[test]
    fn ssh_config_include_006_a_lab_folder_with_a_quote_is_not_listed() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");

        assert!(add_lab_ssh_include_at(&ssh_dir, r#"/tmp/a"b/.sherpa/ssh/config"#).is_err());

        assert!(!read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE)).contains("/tmp/a"));
    }

    #[cfg(unix)]
    #[test]
    fn ssh_config_include_007_a_symlinked_ssh_config_keeps_its_link_and_mode() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let dotfiles = tmp.path().join("dotfiles_ssh_config");
        std::fs::write(&dotfiles, "Host myserver\n").unwrap();
        std::fs::set_permissions(&dotfiles, Permissions::from_mode(0o600)).unwrap();
        symlink(&dotfiles, ssh_dir.join("config")).unwrap();

        add_lab_ssh_include_at(&ssh_dir, "/tmp/lab1/.sherpa/ssh/config").unwrap();

        assert!(
            std::fs::symlink_metadata(ssh_dir.join("config"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(read_file(&dotfiles).contains("Include "));
        let mode = std::fs::metadata(&dotfiles).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn ssh_config_include_008_labs_listed_at_the_same_moment_are_all_listed() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::thread::scope(|scope| {
            for lab in 0..16 {
                let ssh_dir = &ssh_dir;
                scope.spawn(move || {
                    add_lab_ssh_include_at(ssh_dir, &format!("/tmp/lab{lab}/.sherpa/ssh/config"))
                        .unwrap();
                });
            }
        });
        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        for lab in 0..16 {
            assert!(
                content.contains(&format!("/tmp/lab{lab}/.sherpa/ssh/config")),
                "{content}"
            );
        }
        assert_eq!(
            read_file(&ssh_dir.join("config"))
                .matches("Include ")
                .count(),
            1
        );
    }

    #[test]
    fn stale_includes_002_a_line_whose_lab_the_server_lists_is_valid() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let include_path = create_lab_files(tmp.path(), "lab1", "lab-active");
        add_lab_ssh_include_at(&ssh_dir, &include_path).unwrap();

        let mut active = BTreeSet::new();
        active.insert("lab-active".to_string());
        let report = inspect_lab_ssh_includes_at(&ssh_dir, Some(&active), None).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].status, SshConfigEntryStatus::Valid);
        assert_eq!(report.entries[0].lab_id.as_deref(), Some("lab-active"));
    }

    #[test]
    fn stale_includes_003_a_line_whose_lab_the_server_does_not_list_is_stale() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let include_path = create_lab_files(tmp.path(), "lab1", "deleted-lab");
        add_lab_ssh_include_at(&ssh_dir, &include_path).unwrap();

        let active = BTreeSet::new();
        let report = inspect_lab_ssh_includes_at(&ssh_dir, Some(&active), None).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].status, SshConfigEntryStatus::Stale);
        assert_eq!(report.entries[0].lab_id.as_deref(), Some("deleted-lab"));
    }

    #[test]
    fn stale_includes_004_a_line_whose_ssh_config_is_gone_is_stale() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/tmp/missing/.sherpa/ssh/config").unwrap();

        let report = inspect_lab_ssh_includes_at(&ssh_dir, None, None).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].status, SshConfigEntryStatus::Stale);
    }

    #[test]
    fn stale_includes_006_a_line_for_a_file_that_is_not_a_lab_s_ssh_config_is_broken() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        let lab_dir = tmp.path().join("lab1");
        std::fs::create_dir_all(lab_dir.join(".sherpa/ssh")).unwrap();
        std::fs::create_dir_all(lab_dir.join("other/ssh")).unwrap();
        std::fs::write(lab_dir.join("lab-info.toml"), valid_lab_info_toml("old")).unwrap();
        std::fs::write(
            lab_dir.join(".sherpa/lab-info.toml"),
            valid_lab_info_toml("old"),
        )
        .unwrap();
        let others = [
            "sherpa_ssh_config",
            ".sherpa/config",
            ".sherpa/ssh/key",
            "other/ssh/config",
        ]
        .map(|other| lab_dir.join(other));
        for other in &others {
            std::fs::write(other, "Host test\n").unwrap();
            add_lab_ssh_include_at(&ssh_dir, &other.to_string_lossy()).unwrap();
        }

        let report = inspect_lab_ssh_includes_at(&ssh_dir, None, None).unwrap();

        assert_eq!(report.entries.len(), others.len());
        for entry in &report.entries {
            assert_eq!(entry.status, SshConfigEntryStatus::Broken, "{entry:?}");
            assert!(entry.reason.contains(".sherpa/ssh/config"), "{entry:?}");
        }
    }

    #[test]
    fn lab_folder_upgrade_010_with_no_include_for_the_lab_nothing_in_ssh_is_written() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        assert!(!replace_lab_ssh_include(&ssh_dir, "/labs/a/sherpa_ssh_config", "/x").unwrap());
        assert!(!ssh_dir.exists(), "no index, so nothing is created");

        add_lab_ssh_include_at(&ssh_dir, "/labs/b/sherpa_ssh_config").unwrap();
        let before = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert!(!replace_lab_ssh_include(&ssh_dir, "/labs/a/sherpa_ssh_config", "/x").unwrap());
        assert_eq!(read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE)), before);
    }

    #[test]
    fn lab_folder_upgrade_009_an_include_present_at_both_paths_is_left_once_at_the_new_one() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        add_lab_ssh_include_at(&ssh_dir, "/labs/a/.sherpa/ssh/config").unwrap();
        add_lab_ssh_include_at(&ssh_dir, "/labs/a/sherpa_ssh_config").unwrap();

        replace_lab_ssh_include(
            &ssh_dir,
            "/labs/a/sherpa_ssh_config",
            "/labs/a/.sherpa/ssh/config",
        )
        .unwrap();

        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert_eq!(content.matches("Include ").count(), 1, "{content}");
        assert!(content.contains("/labs/a/.sherpa/ssh/config"));
    }

    #[test]
    fn stale_includes_005_a_line_whose_lab_info_cannot_be_read_is_broken() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let lab_folder = LabFolder::new(&tmp.path().join("lab1"));
        std::fs::create_dir_all(lab_folder.ssh_dir()).unwrap();
        let ssh_config_path = lab_folder.ssh_config();
        std::fs::write(&ssh_config_path, "Host test\n").unwrap();
        std::fs::write(lab_folder.lab_info(), "not = valid = toml").unwrap();
        add_lab_ssh_include_at(&ssh_dir, &ssh_config_path.to_string_lossy()).unwrap();

        let report = inspect_lab_ssh_includes_at(&ssh_dir, None, None).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].status, SshConfigEntryStatus::Broken);
    }

    #[test]
    fn stale_includes_008_clean_removes_a_stale_line_and_keeps_a_valid_one() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let valid_include = create_lab_files(tmp.path(), "lab1", "active-lab");
        let stale_include = create_lab_files(tmp.path(), "lab2", "deleted-lab");
        add_lab_ssh_include_at(&ssh_dir, &valid_include).unwrap();
        add_lab_ssh_include_at(&ssh_dir, &stale_include).unwrap();

        let mut active = BTreeSet::new();
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
    fn stale_includes_007_a_second_line_for_the_same_file_is_stale_and_clean_keeps_one() {
        let tmp = TempDir::new().unwrap();
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let include_path = create_lab_files(tmp.path(), "lab1", "active-lab");
        ensure_sherpa_include_in_ssh_config_at(&ssh_dir).unwrap();
        std::fs::write(
            ssh_dir.join(SHERPA_SSH_INDEX_FILE),
            format!("{SHERPA_SSH_INDEX_HEADER}\nInclude {include_path}\nInclude {include_path}\n"),
        )
        .unwrap();

        let mut active = BTreeSet::new();
        active.insert("active-lab".to_string());
        let report = clean_stale_lab_ssh_includes_at(&ssh_dir, Some(&active), None).unwrap();

        assert_eq!(report.kept.len(), 1);
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].status, SshConfigEntryStatus::Stale);
        let content = read_file(&ssh_dir.join(SHERPA_SSH_INDEX_FILE));
        assert_eq!(content.matches(&include_path).count(), 1);
    }
}
