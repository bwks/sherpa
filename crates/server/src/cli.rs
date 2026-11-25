use std::net::{Ipv4Addr, Ipv6Addr};

use clap::{Parser, Subcommand};
use ipnet::{Ipv4Net, Ipv6Net};

use shared::konst::{
    SHERPA_MANAGEMENT_NETWORK_IPV4, SHERPA_SERVER_HTTP_PORT, SHERPA_SERVER_WS_PORT,
};

use crate::konst::{SHERPA_MANAGEMENT_NETWORK_IPV6, SHERPA_SERVER_IPV4};

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
        /// Overwrite the existing config
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,

        /// Server listen IPv4 address
        #[arg(long = "server-ipv4", env = "SHERPA_SERVER_IPV4", default_value = SHERPA_SERVER_IPV4)]
        server_ipv4: Ipv4Addr,

        /// Server listen IPv6 address, if the server also listens on IPv6
        #[arg(long = "server-ipv6", env = "SHERPA_SERVER_IPV6")]
        server_ipv6: Option<Ipv6Addr>,

        /// Server listen port
        #[arg(long = "ws-port", env = "SHERPA_SERVER_WS_PORT", default_value_t = SHERPA_SERVER_WS_PORT)]
        ws_port: u16,

        /// HTTP port for the certificate endpoint
        #[arg(long = "http-port", env = "SHERPA_SERVER_HTTP_PORT", default_value_t = SHERPA_SERVER_HTTP_PORT)]
        http_port: u16,

        /// IPv4 prefix that lab management /24 subnets are taken from. Like the other server
        /// settings it is written only with a new config: an existing config is kept, and
        /// --force rewrites it
        #[arg(
            long = "management-prefix-ipv4",
            env = "SHERPA_MANAGEMENT_PREFIX_IPV4",
            default_value_t = SHERPA_MANAGEMENT_NETWORK_IPV4
        )]
        management_prefix_ipv4: Ipv4Net,

        #[arg(
            long = "management-prefix-ipv6",
            env = "SHERPA_MANAGEMENT_PREFIX_IPV6",
            help = format!(
                "IPv6 prefix that lab management /64 subnets are taken from. Left out, the \
                 config has none and labs use {SHERPA_MANAGEMENT_NETWORK_IPV6}. Written only \
                 with a new config, like the IPv4 prefix"
            )
        )]
        management_prefix_ipv6: Option<Ipv6Net>,
    },

    /// Check, without changing it, that this release can use the server's database. It names
    /// the labs to destroy first, if any
    CheckDatabase,

    /// Fix up server environment
    Doctor {
        /// Set node image permissions
        #[arg(long, action = clap::ArgAction::SetTrue)]
        images: bool,
    },

    /// Manage accounts in the database of a stopped server. Run as the sherpa user
    User {
        #[clap(subcommand)]
        command: UserCommands,
    },
}

#[derive(Subcommand)]
pub enum UserCommands {
    /// Unlock an account that failed logins locked, the built-in sherpa-admin included
    ///
    /// Sets the account's failed login count back to 0. sherpad holds the database while it
    /// runs, so stop it first, and run this as the sherpa user:
    /// sudo -u sherpa sherpad user unlock sherpa-admin
    Unlock {
        /// Username of the account to unlock
        username: String,
    },
}
