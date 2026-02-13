use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "sherpad")]
#[command(bin_name = "sherpad")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Sherpa Server Daemon", long_about = None)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the sherpad server
    Start {
        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,
    },

    /// Stop the sherpad server
    Stop {
        /// Force kill if graceful shutdown fails
        #[arg(short, long)]
        force: bool,
    },

    /// Restart the sherpad server
    Restart {
        /// Run in foreground after restart
        #[arg(short, long)]
        foreground: bool,
    },

    /// Show sherpad server status
    Status,

    /// Show sherpad server logs
    Logs {
        /// Follow log output (like tail -f)
        #[arg(short, long)]
        follow: bool,
    },
}
