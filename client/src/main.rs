mod cmd;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match cmd::Cli::run().await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            eprintln!("{:?}", e);
            // match e.source() {
            //     Some(s) => {
            //         eprintln!("{s}");
            //         // event!(target: APP_NAME, Level::ERROR, "{s}")
            //     }
            //     None => {
            //         eprintln!("{e}");
            //         // event!(target: APP_NAME, Level::ERROR, "{e}")
            //     }
            // }
            ExitCode::from(1)
        }
    }
}
