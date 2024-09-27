use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};

use crate::core::konst::{BOOTSTRAP_ISO, CONFIG_DIR, TEMP_DIR};

/// Create a directory, expanding ~ if it's passed
pub fn create_dir(dir_path: &str) -> Result<()> {
    fs::create_dir_all(dir_path)?;
    println!("Directory path created successfully: {dir_path}");

    Ok(())
}

/// Create a file, expanding ~ if it's passed
pub fn create_file(file_path: &str, contents: String) -> Result<()> {
    fs::write(file_path, contents)?;
    println!("File created successfully: {file_path}");

    Ok(())
}

/// Check for file existence, expanding ~ if it's passed
pub fn file_exists(file_path: &str) -> bool {
    let path = Path::new(file_path);
    path.exists() && path.is_file()
}

/// Check for directory existence, expanding ~ if it's passed
pub fn dir_exists(dir_path: &str) -> bool {
    let path = Path::new(dir_path);
    path.exists() && path.is_dir()
}

/// Expand a path if it's passed with ~
pub fn expand_path(path: &str) -> String {
    let expanded_path = shellexpand::tilde(path);
    let full_path = Path::new(expanded_path.as_ref());
    full_path.display().to_string()
}

/// Copy a file from a source to a destination.
/// This will overwrite the destination file if it exists.
pub fn copy_file(src: &str, dst: &str) -> Result<()> {
    let source = File::open(src)?;
    let destination = File::create(dst)?;

    let mut reader = BufReader::new(source);
    let mut writer = BufWriter::new(destination);

    io::copy(&mut reader, &mut writer)?;

    writer.flush()?; // Ensures all buffered contents are written to the file

    Ok(())
}

/// Read an SSH public key file and return a String.
pub fn get_ssh_public_key(path: &str) -> Result<String> {
    let full_path = expand_path(path);
    let file = File::open(&full_path)?;
    let reader = BufReader::new(file);

    // Read the first line of the file
    if let Some(line) = reader.lines().next() {
        let key = line?;
        Ok(key)
    } else {
        Err(anyhow::anyhow!("Invalid SSH public key file: {full_path}",))
    }
}

/// Set permissions for all files in a folder subtree.
/// Sets files to read-only and removes executable bit for users/groups.
pub fn fix_permissions_recursive(path: &str) -> Result<()> {
    let path = expand_path(path);

    let metadata = fs::metadata(&path)?;
    let mut perms = metadata.permissions();

    if metadata.is_dir() {
        // Set directory permissions to 0755
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;

        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            fix_permissions_recursive(
                entry
                    .path()
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Error updating read-only permissions"))?,
            )?;
        }
    } else {
        // Set file permissions to 0444
        perms.set_mode(0o440);
        fs::set_permissions(path, perms)?;
    }

    Ok(())
}

/// Convert an SSH public key string to an MD5 Hash.
/// Expected key format:
///  - <key-type> <key> [optional-description]
///  - ssh-rsa ABCDEF12345... [user@host key]
pub fn pub_ssh_key_to_md5_hash(pub_key_str: &str) -> Result<String> {
    let key_parts: Vec<&str> = pub_key_str.split_whitespace().collect();

    // Validate that the key contains at least two parts (type and base64 key)
    if key_parts.len() < 2 {
        return Err(anyhow!("Invalid SSH public key format"));
    }

    let base64_key = key_parts[1];

    // Decode the base64 encoded key to get binary data
    let binary_key = general_purpose::STANDARD
        .decode(base64_key)
        .map_err(|e| anyhow!("Error decoding base64 key: {}", e))?;

    // Compute the MD5 hash of the binary key data
    let md5_hash = md5::compute(binary_key);

    // Format the hash as an uppercase hexadecimal string accepted by cisco devices
    let formatted_hash = format!("{:X}", md5_hash);

    Ok(formatted_hash)
}

/// Create a bootstrap ISO file.
/// This wraps the `genisoimage` command to create a bootstrap ISO file.
/// `genisoimage` must be installed on the system.
///
/// DOGWATER:
/// This function creates temp directory and files in the /tmp
/// directory. This is dogwater, and in the future, the plan is to
/// implement this functionality in pure Rust.
pub fn create_bootstrap_iso(name: String, bootstrap_files: Vec<String>) -> Result<()> {
    // Create a temporary directory
    fs::create_dir_all(format!("{TEMP_DIR}/{name}"))?;

    let iso_path = expand_path(&format!("{CONFIG_DIR}/{BOOTSTRAP_ISO}"));

    // Create ISO using genisoimage
    Command::new("genisoimage")
        .args([
            "-output",
            &iso_path,
            "-volid",
            "cidata",
            "-joliet",
            "-rock",
            "--input-charset",
            "utf-8",
            &bootstrap_files
                .iter()
                .map(|x| format!("{TEMP_DIR}/{x}"))
                .collect::<Vec<String>>()
                .join(" "),
        ])
        .status()?;
    println!("cloud-init ISO created successfully: {iso_path}");

    // Clean up temp files
    fs::remove_dir_all(format!("{TEMP_DIR}/{name}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pub_ssh_key_to_md5_hash() {
        let test_cases = vec![
            (
                "ssh-rsa AAAAB3NzaC1yc2EAAAABIwAAAQEA6NF8iallvQVp22WDkTkyrtvp9eWW6A8YVr+kz4TjGYe7gHzIw+niNltGEFHzD8+v1I2YJ6oXevct1YeS0o9HZyN1Q9qgCgzUFtdOKLv6IedplqoPkcmF0aYet2PkEDo3MlTBckFXPITAMzF8dJSIFo9D8HfdOV0IAdx4O7PtixWKn5y2hMNG0zQPyUecp4pzC6kivAIhyfHilFR61RGL+GPXQ2MWZWFYbAGjyiYJnAmCP3NOTd0jMZEnDkbUvxhMmBYSdETk1rRgm+R4LOzFUGaHqHDLKLX+FIPKcF96hrucXzcWyLbIbEgE98OHlnVYCzRdK8jlqm8tehUc9c9WhQ== vagrant insecure public key",
                "DD3BB82E850406E9ABFFA80AC0046ED6",
            ),
            // Add more test cases as needed
        ];

        for (input, expected) in test_cases {
            assert_eq!(pub_ssh_key_to_md5_hash(input).unwrap(), expected);
        }
    }

    #[test]
    fn test_pub_ssh_key_to_md5_hash_invalid_input() {
        let invalid_key = "not a valid ssh key";
        assert!(pub_ssh_key_to_md5_hash(invalid_key).is_err());
    }
}
