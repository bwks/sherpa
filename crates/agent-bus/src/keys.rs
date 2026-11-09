//! `NKey` user keys. An agent proves who it is by signing the server's challenge with its seed.
//! The seed is a secret: it is read only from the environment, and is never written to a file
//! or a log, or put in an error message. The public key is not secret.

use std::env;

use anyhow::{Result, anyhow, ensure};
use nkeys::{KeyPair, KeyPairType};

/// The environment variable that holds the agent's seed.
pub const SEED_VARIABLE: &str = "AGENT_BUS_SEED";

/// A new user key pair: the public key goes in the agents file, the seed to its agent.
pub struct NewKey {
    pub public_key: String,
    pub seed: String,
}

/// Generate a new user key pair.
///
/// # Errors
///
/// Fails when the seed cannot be encoded.
pub fn generate() -> Result<NewKey> {
    let pair = KeyPair::new_user();
    let seed = pair
        .seed()
        .map_err(|error| anyhow!("Unable to encode the new seed: {error}"))?;
    Ok(NewKey {
        public_key: pair.public_key(),
        seed,
    })
}

/// Check that `key` is a user public key. The error never repeats `key`, as a mistake could
/// have put a seed there.
///
/// # Errors
///
/// Fails when `key` is not a valid user public key.
pub fn check_public_key(key: &str) -> Result<()> {
    let pair = KeyPair::from_public_key(key)
        .map_err(|_| anyhow!("The value is not a valid NKey public key"))?;
    ensure!(
        pair.key_pair_type() == KeyPairType::User,
        "The value is not a user public key (it must start with U)"
    );
    Ok(())
}

/// The public key of a user `seed`. The error never repeats the seed.
///
/// # Errors
///
/// Fails when `seed` is not a user seed.
pub fn public_key_of(seed: &str) -> Result<String> {
    let pair = KeyPair::from_seed(seed.trim())
        .map_err(|_| anyhow!("AGENT_BUS_SEED is not a valid NKey seed"))?;
    ensure!(
        pair.key_pair_type() == KeyPairType::User,
        "AGENT_BUS_SEED is not a user seed (it must start with SU)"
    );
    Ok(pair.public_key())
}

/// The agent's seed, read from `AGENT_BUS_SEED`.
///
/// # Errors
///
/// Fails, naming the variable, when it is unset, empty or not a user seed.
pub fn seed_from_environment() -> Result<String> {
    checked_seed(env::var(SEED_VARIABLE).ok())
}

fn checked_seed(value: Option<String>) -> Result<String> {
    let seed = value
        .map(|seed| seed.trim().to_owned())
        .filter(|seed| !seed.is_empty())
        .ok_or_else(|| {
            anyhow!("Set {SEED_VARIABLE} to this agent's NKey seed (see `agent-bus enroll` for a guest, `agent-bus key new` for the host)")
        })?;
    public_key_of(&seed)?;
    Ok(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_invalid_seed_names_the_variable_and_never_echoes_it() {
        for value in [None, Some("  ".to_owned())] {
            let error = checked_seed(value).unwrap_err().to_string();
            assert!(error.contains("AGENT_BUS_SEED"));
        }
        let bad = "SUNOTAREALSEED".to_owned();
        let error = checked_seed(Some(bad.clone())).unwrap_err().to_string();
        assert!(error.contains("AGENT_BUS_SEED"));
        assert!(!error.contains(&bad));
        let good = generate().unwrap().seed;
        assert_eq!(checked_seed(Some(format!(" {good}\n"))).unwrap(), good);
    }

    #[test]
    fn a_new_key_is_a_user_key_whose_seed_gives_the_same_public_key() {
        let key = generate().unwrap();
        assert!(key.public_key.starts_with('U'));
        assert!(key.seed.starts_with("SU"));
        check_public_key(&key.public_key).unwrap();
        assert_eq!(public_key_of(&key.seed).unwrap(), key.public_key);
    }

    #[test]
    fn only_user_public_keys_pass() {
        let account = KeyPair::new_account().public_key();
        assert!(check_public_key(&account).is_err());
        assert!(check_public_key("UNOTAKEY").is_err());
    }

    #[test]
    fn errors_never_repeat_a_seed() {
        let seed = generate().unwrap().seed;
        let error = check_public_key(&seed).unwrap_err().to_string();
        assert!(!error.contains(&seed));
        let wrong = KeyPair::new_account().seed().unwrap();
        let error = public_key_of(&wrong).unwrap_err().to_string();
        assert!(!error.contains(&wrong));
    }
}
