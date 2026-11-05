//! Vault nodes Sherpa initialises and unseals.
//!
//! Sherpa initialises each Vault node that runs its generated config, keeps the unseal key and
//! root token with the lab's files, unseals it, and hands the credentials to the user.

pub mod api;
pub mod lab;

pub use lab::{ensure_vault_unsealed, read_vault_credentials, sherpa_initialises_vault};
