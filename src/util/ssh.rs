use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};

use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;

use crate::core::konst::{SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_SSH_PUBLIC_KEY_FILE};
use crate::model::{SshKeyAlgorithms, SshPublicKey};
use crate::util::{create_file, expand_path};

/// Read an SSH public key file and return a String.
pub fn get_ssh_public_key(path: &str) -> Result<SshPublicKey> {
    let full_path = expand_path(path);
    let file = File::open(&full_path)?;
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
pub fn generate_ssh_keypair(directory: &str) -> Result<()> {
    let mut rng = OsRng;

    // Generate a new private key
    let private_key = PrivateKey::random(
        &mut rng,
        Algorithm::Rsa {
            hash: Some(HashAlg::Sha256),
        },
        // Algorithm::Ed25519,
    )?;

    // Serialize the private key to the OpenSSH format
    let private_key_pem = private_key.to_openssh(LineEnding::LF)?;

    // Extract the public key from the private key
    let public_key = private_key.public_key();
    let public_key_ssh = public_key.to_openssh()?;

    let private_key_path = &format!("{directory}/{SHERPA_SSH_PRIVATE_KEY_FILE}");
    let public_key_path = &format!("{directory}/{SHERPA_SSH_PUBLIC_KEY_FILE}");

    // Create the SSH Public/Private keypair
    create_file(public_key_path, public_key_ssh.to_string())?;
    create_file(private_key_path, private_key_pem.to_string())?;

    // Update permissions of private key file to be read-only.
    let metadata = fs::metadata(private_key_path)?;
    let mut perms = metadata.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(private_key_path, perms)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
