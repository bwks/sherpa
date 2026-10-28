#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), deny(unsafe_code))]

mod cluster_credentials;
mod cmd;
mod lab_files;
mod private_key;
mod provision;
mod ssh_client;
mod token;
mod ws_client;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_acl;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    // Initialize rustls crypto provider (required for rustls 0.23+)
    // This must be done before any rustls operations
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    match cmd::Cli::run().await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            eprintln!("{:?}", e);
            ExitCode::from(1)
        }
    }
}
