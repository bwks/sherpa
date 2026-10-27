#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), deny(unsafe_code))]

mod cluster_credentials;
mod cmd;
mod private_key;
mod provision;
mod remote_shell;
mod ssh_client;
mod token;
mod ws_client;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_acl;
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_console;

use std::process::ExitCode;
use std::thread;

use tokio::runtime::Builder;

/// Stack size for the thread commands run on and the runtime's threads: 8 MiB, as Linux gives
/// a main thread. Windows gives its main thread 1 MiB, which an SSH connection's nested polls
/// overflow in a debug build.
const THREAD_STACK_SIZE: usize = 8 * 1024 * 1024;

fn main() -> ExitCode {
    // Initialize rustls crypto provider (required for rustls 0.23+)
    // This must be done before any rustls operations
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let commands = thread::Builder::new()
        .name("sherpa".to_string())
        .stack_size(THREAD_STACK_SIZE)
        .spawn(run_commands);
    match commands {
        Ok(handle) => match handle.join() {
            Ok(code) => code,
            Err(_) => {
                eprintln!("sherpa stopped unexpectedly");
                ExitCode::from(1)
            }
        },
        Err(error) => {
            eprintln!("Failed to start: {error}");
            ExitCode::from(1)
        }
    }
}

/// Run the command line on an async runtime whose threads have the same stack size.
fn run_commands() -> ExitCode {
    let runtime = match Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(THREAD_STACK_SIZE)
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Failed to start the async runtime: {error}");
            return ExitCode::from(1);
        }
    };
    match runtime.block_on(cmd::Cli::run()) {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            eprintln!("{:?}", e);
            ExitCode::from(1)
        }
    }
}
