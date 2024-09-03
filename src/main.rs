mod cmd;
mod core;
mod libvirt;
mod model;
mod topology;
mod util;

use std::process::ExitCode;

use cmd::Cli;

#[tokio::main]
async fn main() -> ExitCode {
    match Cli::run().await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            match e.source() {
                Some(s) => {
                    eprintln!("{s}");
                    // event!(target: APP_NAME, Level::ERROR, "{s}")
                }
                None => {
                    eprintln!("{e}");
                    // event!(target: APP_NAME, Level::ERROR, "{e}")
                }
            }
            ExitCode::from(1)
        }
    }
}
