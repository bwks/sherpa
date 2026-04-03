#![deny(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

pub mod api;
pub mod auth;
pub mod cli;
pub mod daemon;
pub mod doctor;
pub mod init;
pub mod services;
pub mod templates;
pub mod tls;
