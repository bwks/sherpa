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

    use tempfile::TempDir;

    use crate::services::keycloak::lab::{ensure_keycloak_credentials, read_keycloak_credentials};
    use crate::services::surreal_db::{ensure_surreal_db_credentials, read_surreal_db_credentials};

    const KEYCLOAK: &str = "https://172.31.1.23:8443";
    const SURREAL_DB: &str = "http://172.31.1.24:8000";

    #[test]
    fn lab_credentials_011_a_password_is_20_mixed_letters_and_digits_and_new_each_time() {
        let password = random_password();
        assert_eq!(password.len(), 20);
        assert!(password.chars().all(|c| c.is_ascii_alphanumeric()));
        assert!(password.chars().any(|c| c.is_ascii_lowercase()));
        assert!(password.chars().any(|c| c.is_ascii_uppercase()));
        assert!(password.chars().any(|c| c.is_ascii_digit()));
        assert_ne!(password, random_password());
    }

    #[test]
    fn lab_credentials_012_two_nodes_of_one_lab_get_different_passwords() {
        let lab = TempDir::new().unwrap();
        let keycloaks = lab.path().join("keycloak");
        let surreal_dbs = lab.path().join("surrealdb");

        let passwords = [
            ensure_keycloak_credentials(&keycloaks, "kc01", KEYCLOAK, "alice").unwrap(),
            ensure_keycloak_credentials(&keycloaks, "kc02", KEYCLOAK, "alice").unwrap(),
            ensure_surreal_db_credentials(&surreal_dbs, "db01", SURREAL_DB).unwrap(),
            ensure_surreal_db_credentials(&surreal_dbs, "db02", SURREAL_DB).unwrap(),
        ]
        .map(|admin| admin.password);

        for (index, password) in passwords.iter().enumerate() {
            assert!(!passwords[index + 1..].contains(password), "{index}");
        }
    }

    #[test]
    fn lab_credentials_013_a_redeployed_node_keeps_its_saved_administrator_and_users() {
        let lab = TempDir::new().unwrap();
        let keycloaks = lab.path().join("keycloak");
        let surreal_dbs = lab.path().join("surrealdb");
        let keycloak = ensure_keycloak_credentials(&keycloaks, "kc01", KEYCLOAK, "alice").unwrap();
        let surreal_db = ensure_surreal_db_credentials(&surreal_dbs, "db01", SURREAL_DB).unwrap();
        let saved_keycloaks = read_keycloak_credentials(&keycloaks).unwrap();
        let saved_surreal_dbs = read_surreal_db_credentials(&surreal_dbs).unwrap();

        let keycloak_again =
            ensure_keycloak_credentials(&keycloaks, "kc01", KEYCLOAK, "alice").unwrap();
        let surreal_db_again =
            ensure_surreal_db_credentials(&surreal_dbs, "db01", SURREAL_DB).unwrap();

        assert_eq!(keycloak_again, keycloak);
        assert_eq!(surreal_db_again, surreal_db);
        assert_eq!(
            read_keycloak_credentials(&keycloaks).unwrap(),
            saved_keycloaks
        );
        assert_eq!(
            read_surreal_db_credentials(&surreal_dbs).unwrap(),
            saved_surreal_dbs
        );
    }
}
