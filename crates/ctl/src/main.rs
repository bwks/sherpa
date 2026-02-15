mod cmd;
mod common;
mod token;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match cmd::Cli::run().await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::from(1)
        }
    }
}
