//! Passwords Sherpa generates for the logins it makes on application nodes: a Keycloak node's
//! admin and realm users, and a `SurrealDB` node's root user. Each node gets its own, kept in a
//! file only the server can read in the lab directory, and handed to the lab owner by `up` and
//! `download`.

use rand::Rng;
use rand::distributions::{Alphanumeric, DistString};
use rand::seq::SliceRandom;

/// Length of a generated password.
const PASSWORD_LENGTH: usize = 20;

/// A random password of [`PASSWORD_LENGTH`] letters and digits, with at least one lowercase
/// letter, one uppercase letter and one digit, which the applications' password policies take.
pub(crate) fn random_password() -> String {
    let mut rng = rand::thread_rng();
    let mut chars: Vec<char> = Alphanumeric
        .sample_string(&mut rng, PASSWORD_LENGTH - 3)
        .chars()
        .collect();
    chars.push(rng.gen_range('a'..='z'));
    chars.push(rng.gen_range('A'..='Z'));
    chars.push(rng.gen_range('0'..='9'));
    chars.shuffle(&mut rng);
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_long_mixed_and_different_each_time() {
        let password = random_password();
        assert_eq!(password.len(), PASSWORD_LENGTH);
        assert!(password.chars().all(|c| c.is_ascii_alphanumeric()));
        assert!(password.chars().any(|c| c.is_ascii_lowercase()));
        assert!(password.chars().any(|c| c.is_ascii_uppercase()));
        assert!(password.chars().any(|c| c.is_ascii_digit()));
        assert_ne!(password, random_password());
    }
}
