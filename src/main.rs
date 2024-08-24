mod cmd;
mod core;
mod topology;

use cmd::Cli;
use core::Config;

fn main() {
    Cli::start();
}
