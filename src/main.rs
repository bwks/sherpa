mod cmd;
mod core;
mod topology;

use std::process::ExitCode;

use cmd::Cli;
use core::Config;

fn main() -> ExitCode {
    match Cli::run() {
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
