#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

pub mod api;
pub mod auth;
pub mod check_database;
pub mod cli;
pub mod daemon;
pub mod doctor;
pub mod init;
pub(crate) mod services;
pub(crate) mod templates;
pub(crate) mod tls;
pub(crate) mod validate;
