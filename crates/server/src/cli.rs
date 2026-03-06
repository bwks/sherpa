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

    /// Initialise the Sherpa server environment
    Init {
        /// Overwrite existing config and keys
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,

        /// SurrealDB password (also reads from SHERPA_DB_PASSWORD env var or /opt/sherpa/env/sherpa.env)
        #[arg(long = "db-pass", env = "SHERPA_DB_PASSWORD")]
        db_password: Option<String>,

        /// Server listen IP address (also reads from SHERPA_SERVER_IP4 env var or /opt/sherpa/env/sherpa.env)
        #[arg(long = "server-ip", env = "SHERPA_SERVER_IP4")]
        server_ip: Option<String>,

        /// Server listen port (also reads from SHERPA_SERVER_PORT env var or /opt/sherpa/env/sherpa.env)
        #[arg(long = "server-port", env = "SHERPA_SERVER_PORT")]
        server_port: Option<u16>,

        /// SurrealDB port (also reads from SHERPA_DB_PORT env var or /opt/sherpa/env/sherpa.env)
        #[arg(long = "db-port", env = "SHERPA_DB_PORT")]
        db_port: Option<u16>,
    },

    /// Fix up server environment
    Doctor {
        /// Set base box permissions to read-only
        #[arg(long, action = clap::ArgAction::SetTrue)]
        boxes: bool,
    },
}
