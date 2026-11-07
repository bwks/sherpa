/// SSH key validation utilities
use anyhow::{Context, Result, bail};
use ssh_key::{Algorithm, PublicKey};

/// The key types the validator accepts, for its error message.
const SUPPORTED_ALGORITHMS: &str = "ssh-ed25519, ssh-rsa, ecdsa-sha2-nistp256, \
     ecdsa-sha2-nistp384, ecdsa-sha2-nistp521, sk-ssh-ed25519@openssh.com, \
     sk-ecdsa-sha2-nistp256@openssh.com";

/// Validates an SSH public key in the OpenSSH one-line format, `algorithm key_data [comment]`.
///
/// The key data is decoded, so a key that only looks right is refused. DSA keys, which OpenSSH
/// no longer supports, are refused, and so is a key that spans more than one line, since it
/// would add a second key wherever it is written to `authorized_keys`.
///
/// # Errors
///
/// Returns an error naming the problem when the key is empty, spans lines, does not decode, or
/// uses an unsupported algorithm.
pub fn validate_ssh_key(key: &str) -> Result<()> {
    let key = key.trim();

    if key.is_empty() {
        bail!("SSH key cannot be empty");
    }
    if key.contains(['\n', '\r']) {
        bail!("SSH public keys must occupy a single line");
    }

    let parsed =
        PublicKey::from_openssh(key).context("SSH key is not a valid OpenSSH public key")?;
    match parsed.algorithm() {
        Algorithm::Ed25519
        | Algorithm::Rsa { .. }
        | Algorithm::Ecdsa { .. }
        | Algorithm::SkEd25519
        | Algorithm::SkEcdsaSha2NistP256 => Ok(()),
        algorithm => bail!(
            "SSH key algorithm '{algorithm}' is not supported. Supported algorithms: {SUPPORTED_ALGORITHMS}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED25519: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICwUP1LOygXU4kLJLDSCQ68zDWZKYS3scLybuYvujhPD user@host";
    const RSA: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDJsm1P7sS2t8U6gL0iAI0F1YTNv3GPZNM5ps7oNurGTQE790Sl9tYQLq5bQjlIadMplhQwOr/ue6nRrGlHAueR+WK8G2F1PMWuF7bywsX5Pf8A16SSfM1fFDEocALRDz1MLBHZ1oUEAZlgajEFPvQ3t4Z+ZCBL4tOZtjkOioiOdxe6z6yP7r0QKwaLMLWIRnSUWWdiaYJ2Qh66IWsniv3hQXgVMFPQ4biyeLRx6zRM0poZvglXj3QsSOeBRXIdCyFDqqY9pVRdWHoXTC0bsB3qfrxImIV/NZuiRcxTe0jvHmrpbgjCREO5jKastHqT2zTCxeb+VEFTnnLata/1mHMl user@host";
    const ECDSA_P256: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBDvgJMLm+J9nIjtHHzlrcqeoPkc84bjDUfnhbTv11V+Cp5urFgkQLx5FQ5gCif8YMdbGgeSz82LzNd4N+c0oFbg= user@host";
    const ECDSA_P384: &str = "ecdsa-sha2-nistp384 AAAAE2VjZHNhLXNoYTItbmlzdHAzODQAAAAIbmlzdHAzODQAAABhBH69tDqbFP27ESgu2UF9yLyz7G3u6iG8yIhUr4iiYHJMYAkfaPtqR6sgJ2t6VfyKOIGTZeykZZYvvgjcuBSX08PoCa9DLtOGNtkVYli+SoH1dgBlu2R5JUIXqWWQAi6oVw== user@host";
    const ECDSA_P521: &str = "ecdsa-sha2-nistp521 AAAAE2VjZHNhLXNoYTItbmlzdHA1MjEAAAAIbmlzdHA1MjEAAACFBAFn7ZqiUsS5LA6TQ73CGy2S63CnYOSK/7+q8Uu5u+i5jngNF8pnRrc8Md3kDVqMDIgh+yxWhp2LTdUqrfgvxBoeRgHdvOmCIuqbYtZcKRm4y1JIzpAIw3pz4mkHiRwRcaV5QFXrEBh8hYkyiGMw6c2czDDNArv7QoAy9zbZDc1nUj3LoQ== user@host";
    const SECURITY_KEY: &str = "sk-ssh-ed25519@openssh.com AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29tAAAAIGhSTud7Djo7SmlYx8XNkIC4uGVrwnL5oCpsJgyblPbtAAAABHNzaDo= yubikey";

    #[test]
    fn every_supported_algorithm_is_accepted() {
        for key in [
            ED25519,
            RSA,
            ECDSA_P256,
            ECDSA_P384,
            ECDSA_P521,
            SECURITY_KEY,
        ] {
            assert!(validate_ssh_key(key).is_ok(), "{key}");
        }
    }

    #[test]
    fn a_key_without_a_comment_or_with_surrounding_space_is_accepted() {
        let bare = ED25519.trim_end_matches(" user@host");
        assert!(validate_ssh_key(bare).is_ok());
        assert!(validate_ssh_key(&format!("  {ED25519}\n")).is_ok());
    }

    #[test]
    fn test_empty_key() {
        assert!(validate_ssh_key("").is_err());
        assert!(validate_ssh_key("   ").is_err());
    }

    #[test]
    fn test_invalid_algorithm() {
        let key = ED25519.replacen("ssh-ed25519", "invalid-algo", 1);
        assert!(validate_ssh_key(&key).is_err());
    }

    #[test]
    fn test_missing_key_data() {
        assert!(validate_ssh_key("ssh-ed25519").is_err());
    }

    #[test]
    fn test_invalid_base64() {
        assert!(validate_ssh_key("ssh-ed25519 Invalid!@#$%^&*()Characters user@host").is_err());
    }

    #[test]
    fn key_data_that_is_not_a_key_is_refused() {
        let truncated = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICwUP1LOygXU4kLJLDSCQ68z user@host";
        assert!(validate_ssh_key(truncated).is_err());
    }

    #[test]
    fn letters_outside_base64_are_refused() {
        let key = ED25519.replacen("AAAAC3", "ÄÄÄÄC3", 1);
        assert!(validate_ssh_key(&key).is_err());
    }

    #[test]
    fn a_key_spanning_lines_is_refused() {
        assert!(validate_ssh_key(&format!("{ED25519}\nssh-ed25519 AAAA other")).is_err());
        assert!(validate_ssh_key(&ED25519.replace(" user@host", " user\r@host")).is_err());
    }

    #[test]
    fn a_dsa_key_is_refused() {
        let key = "ssh-dss AAAAB3NzaC1kc3MAAACBAP1/U4EddRIpUt9KnC7s5Of2EbdSPO9EAMMeP4C2USZpRV1AIlH7WT2NWPq/xfW6MPbLm1Vs14E7gB00b/JmYLdrmVClpJ+f6AR7ECLCT7up1/63xhv4O1fnxqimFQ8E+4P208UewwI1VBNaFpEy9nXzrith1yrv8iIDGZ3RSAHHAAAAFQCXYFCPFSMLzLKSuYKi64QL8Fgc9QAAAIEA9+GghdabPd7LvKtcNrhXuXmUr7v6OuqC+VdMCz0HgmdRWVeOutRZT+ZxBxCBgLRJFnEj6EwoFhO3zwkyjMim4TwWeotUfI0o4KOuHiuzpnWRbqN/C/ohNWLx+2J6ASQ7zKTxvqhRkImog9/hWuWfBpKLZl6Ae1UlZAFMO/7PSSoAAACBAKKSU2PFl/qOLxIwmBZPPIcJshVe7bVUpFvyl3BbJDow8rXfskl8wO63OzP/qLmcJM0+JbcRU/53JjTuyk31drV2qxhIOsLDC9dGCWj47Y7TyhPdXh/0dthTRBy6bqGtRPxGa7gJov1xm/UuYYXPIUR/3x9MAZvZ5xvE0kYXO+rx user@host";
        let error = validate_ssh_key(key).unwrap_err();
        assert!(error.to_string().contains("ssh-dss"), "{error:#}");
    }
}
