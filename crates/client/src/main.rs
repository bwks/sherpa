#![deny(clippy::unwrap_used, clippy::expect_used)]
#![deny(unsafe_code)]

mod cluster_credentials;
mod cmd;
mod error;
mod keycloak_credentials;
mod lab_files;
mod lab_folder_migration;
mod provision;
mod secret_file;
mod ssh_client;
mod surreal_db_credentials;
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

use rustls::crypto::aws_lc_rs;

/// Run the `sherpa` command line. Commands are polled on the main thread, whose stack is 1 MiB
/// on Windows and usually 8 MiB on Linux. An SSH connection fits in 1 MiB, through a jump host
/// and in a debug build too, because Jato boxes its connection futures.
#[tokio::main]
async fn main() -> ExitCode {
    // Initialize rustls crypto provider (required for rustls 0.23+)
    // This must be done before any rustls operations
    let _ = aws_lc_rs::default_provider().install_default();

    match cmd::Cli::run().await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            // The node's shell has already said why it exited.
            if !e.is::<cmd::ShellExit>() {
                eprintln!("{e:?}");
            }
            ExitCode::from(cmd::exit_status(&e))
        }
    }
}
