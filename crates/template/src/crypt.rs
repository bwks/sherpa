//! SHA-crypt password hashes, in the form node configurations take them.
//!
//! A device that stores a crypt(3) hash gets one, not the plain password: Linux and BSD
//! guests through cloud-init, Flatcar, Arista EOS, Junos, `SONiC`, FRR (`$6$`, SHA-512) and
//! PAN-OS (`$5$`, SHA-256). Each hash has its own random salt, so two renders of one password
//! differ, and tests verify a hash against the password rather than compare it.

use anyhow::{Context, Result, anyhow};
use rand::RngCore;
use sha_crypt::{Algorithm, Params, PasswordHash, PasswordHasher, ShaCrypt};

/// Random bytes in a salt. Encoded, they are 16 characters, the most SHA-crypt reads.
const SALT_BYTES: usize = 12;

/// A SHA-crypt hash algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptHash {
    /// `$5$`, SHA-256-crypt.
    Sha256,
    /// `$6$`, SHA-512-crypt.
    Sha512,
}

/// `password` hashed with `algorithm` and a random salt, at the default 5000 rounds, in the
/// canonical form `$<id>$<salt>$<hash>`: without a `rounds=` field, which not every device
/// reads.
///
/// # Errors
///
/// Fails if the hash cannot be computed or encoded.
pub fn crypt_hash(password: &str, algorithm: CryptHash) -> Result<String> {
    let algorithm = match algorithm {
        CryptHash::Sha256 => Algorithm::Sha256Crypt,
        CryptHash::Sha512 => Algorithm::Sha512Crypt,
    };
    let mut salt = [0u8; SALT_BYTES];
    rand::thread_rng().fill_bytes(&mut salt);
    let hashed = ShaCrypt::new(algorithm, Params::RECOMMENDED)
        .hash_password_with_salt(password.as_bytes(), &salt)
        .map_err(|error| anyhow!("Failed to hash a password: {error}"))?;
    let hashed = hashed.as_password_hash_ref();
    // SHA-crypt writes the rounds first, then the salt and the hash.
    let mut fields = hashed.fields().skip(1);
    let salt = fields.next().context("SHA-crypt hash has no salt")?;
    let hash = fields.next().context("SHA-crypt hash has no hash")?;
    let mut canonical = PasswordHash::from_id(hashed.id())
        .map_err(|error| anyhow!("Failed to encode a password hash: {error}"))?;
    canonical.push_field(salt);
    canonical.push_field(hash);
    Ok(canonical.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "Granite-Owl42!";

    #[test]
    fn hashes_have_the_algorithm_a_salt_and_no_rounds_field() {
        for (algorithm, prefix, length) in [
            (CryptHash::Sha512, "$6$", 86),
            (CryptHash::Sha256, "$5$", 43),
        ] {
            let hashed = crypt_hash(PASSWORD, algorithm).unwrap();
            assert!(hashed.starts_with(prefix), "{hashed}");
            assert!(!hashed.contains("rounds="), "{hashed}");
            let parts: Vec<&str> = hashed.split('$').collect();
            assert_eq!(parts.len(), 4, "{hashed}");
            assert_eq!(parts[2].len(), 16, "{hashed}");
            assert_eq!(parts[3].len(), length, "{hashed}");
        }
    }

    #[test]
    fn each_hash_has_its_own_salt() {
        assert_ne!(
            crypt_hash(PASSWORD, CryptHash::Sha512).unwrap(),
            crypt_hash(PASSWORD, CryptHash::Sha512).unwrap()
        );
    }
}
