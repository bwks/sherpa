use std::net::{Ipv4Addr, Ipv6Addr};

use clap::{Parser, Subcommand};

use shared::konst::{SHERPA_SERVER_HTTP_PORT, SHERPA_SERVER_IPV4, SHERPA_SERVER_WS_PORT};

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
    },

    /// Fix up server environment
    Doctor {
        /// Set base box permissions to read-only
        #[arg(long, action = clap::ArgAction::SetTrue)]
        boxes: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_defaults_to_the_built_in_address_and_ports() {
        let cli = Cli::try_parse_from(["sherpad", "init"]).unwrap();
        let Commands::Init {
            server_ipv4,
            server_ipv6,
            ws_port,
            http_port,
            ..
        } = cli.command
        else {
            panic!("expected init");
        };
        assert_eq!(server_ipv4, Ipv4Addr::UNSPECIFIED);
        assert_eq!(server_ipv6, None);
        assert_eq!(ws_port, 3030);
        assert_eq!(http_port, 3031);
    }

    #[test]
    fn init_takes_the_address_and_ports_from_flags() {
        let cli = Cli::try_parse_from([
            "sherpad",
            "init",
            "--server-ipv4",
            "192.0.2.10",
            "--server-ipv6",
            "2001:db8::10",
            "--ws-port",
            "4030",
            "--http-port",
            "4031",
        ])
        .unwrap();
        let Commands::Init {
            server_ipv4,
            server_ipv6,
            ws_port,
            http_port,
            ..
        } = cli.command
        else {
            panic!("expected init");
        };
        assert_eq!(server_ipv4, Ipv4Addr::new(192, 0, 2, 10));
        assert_eq!(
            server_ipv6,
            Some("2001:db8::10".parse::<Ipv6Addr>().unwrap())
        );
        assert_eq!(ws_port, 4030);
        assert_eq!(http_port, 4031);
    }

    #[test]
    fn init_rejects_an_invalid_address() {
        assert!(Cli::try_parse_from(["sherpad", "init", "--server-ipv4", "192.0.2"]).is_err());
        assert!(Cli::try_parse_from(["sherpad", "init", "--server-ipv6", "2001:db8::zz"]).is_err());
    }
}
