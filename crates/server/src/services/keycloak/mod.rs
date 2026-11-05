//! Keycloak nodes Sherpa sets up a realm on.
//!
//! On each `keycloak_server` node, Sherpa makes the `sherpa` realm: a `kubernetes` client that
//! Kubernetes clusters accept tokens for, groups bound to cluster roles, and users. The users'
//! passwords are kept with the lab's files and handed to the user.

pub mod api;
pub mod lab;

pub use lab::{ensure_keycloak_realm, keycloak_dir, read_keycloak_credentials};
