#![deny(clippy::unwrap_used, clippy::expect_used)]
#![deny(unsafe_code)]

mod cluster_credentials;
mod cmd;
mod keycloak_credentials;
mod lab_files;
mod lab_folder_migration;
mod private_key;
mod provision;
mod ssh_client;
mod table;
mod token;
mod vault_credentials;
mod ws_client;

#[cfg(windows)]
#[expect(
    unsafe_code,
    reason = "the Win32 security API has no safe Rust wrapper; every call has a SAFETY comment"
)]
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
            if let Some(exit) = e.downcast_ref::<cmd::ShellExit>() {
                ExitCode::from(exit.0)
            } else {
                eprintln!("{e:?}");
                ExitCode::from(1)
            }
        }
    }
}
