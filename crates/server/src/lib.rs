#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

pub mod api;
pub mod auth;
pub mod check_database;
pub mod cli;
pub mod daemon;
pub mod doctor;
pub mod host_account;
pub mod init;
pub(crate) mod konst;
#[cfg(test)]
mod log_capture;
pub(crate) mod services;
pub(crate) mod templates;
pub(crate) mod tls;
pub mod user;
pub(crate) mod validate;

pub use services::error::{ServiceCategory, ServiceError};
