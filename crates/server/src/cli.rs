use std::net::{Ipv4Addr, Ipv6Addr};

use clap::{Parser, Subcommand};
use ipnet::{Ipv4Net, Ipv6Net};

use shared::konst::{
    SHERPA_MANAGEMENT_NETWORK_IPV4, SHERPA_MANAGEMENT_NETWORK_IPV6, SHERPA_SERVER_HTTP_PORT,
    SHERPA_SERVER_IPV4, SHERPA_SERVER_WS_PORT,
};

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
        /// Set base box permissions to read-only
        #[arg(long, action = clap::ArgAction::SetTrue)]
        boxes: bool,
    },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn check_database_takes_no_arguments() {
        let cli = Cli::try_parse_from(["sherpad", "check-database"]).unwrap();
        assert!(matches!(cli.command, Commands::CheckDatabase));
        assert!(Cli::try_parse_from(["sherpad", "check-database", "extra"]).is_err());
    }

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

    #[test]
    fn init_defaults_to_the_built_in_management_prefix() {
        let cli = Cli::try_parse_from(["sherpad", "init"]).unwrap();
        let Commands::Init {
            management_prefix_ipv4,
            ..
        } = cli.command
        else {
            panic!("expected init");
        };
        assert_eq!(management_prefix_ipv4, SHERPA_MANAGEMENT_NETWORK_IPV4);
    }

    #[test]
    fn init_takes_the_management_prefix_from_the_flag() {
        let cli = Cli::try_parse_from([
            "sherpad",
            "init",
            "--management-prefix-ipv4",
            "172.30.0.0/16",
        ])
        .unwrap();
        let Commands::Init {
            management_prefix_ipv4,
            ..
        } = cli.command
        else {
            panic!("expected init");
        };
        assert_eq!(
            management_prefix_ipv4,
            "172.30.0.0/16".parse::<Ipv4Net>().unwrap()
        );
    }

    #[test]
    fn init_reads_the_management_prefix_from_the_environment() {
        let command = Cli::command();
        let init = command.find_subcommand("init").unwrap();
        let argument = init
            .get_arguments()
            .find(|argument| argument.get_id() == "management_prefix_ipv4")
            .unwrap();
        assert_eq!(argument.get_env().unwrap(), "SHERPA_MANAGEMENT_PREFIX_IPV4");
    }

    #[test]
    fn init_leaves_the_ipv6_management_prefix_unset_by_default() {
        let cli = Cli::try_parse_from(["sherpad", "init"]).unwrap();
        let Commands::Init {
            management_prefix_ipv6,
            ..
        } = cli.command
        else {
            panic!("expected init");
        };
        assert_eq!(management_prefix_ipv6, None);
    }

    #[test]
    fn init_takes_the_ipv6_management_prefix_from_the_flag() {
        let cli = Cli::try_parse_from([
            "sherpad",
            "init",
            "--management-prefix-ipv6",
            "fd00:b00a::/48",
        ])
        .unwrap();
        let Commands::Init {
            management_prefix_ipv6,
            ..
        } = cli.command
        else {
            panic!("expected init");
        };
        assert_eq!(
            management_prefix_ipv6,
            Some("fd00:b00a::/48".parse::<Ipv6Net>().unwrap())
        );
    }

    #[test]
    fn init_reads_the_ipv6_management_prefix_from_the_environment() {
        let command = Cli::command();
        let init = command.find_subcommand("init").unwrap();
        let argument = init
            .get_arguments()
            .find(|argument| argument.get_id() == "management_prefix_ipv6")
            .unwrap();
        assert_eq!(argument.get_env().unwrap(), "SHERPA_MANAGEMENT_PREFIX_IPV6");
    }

    #[test]
    fn init_rejects_an_invalid_ipv6_management_prefix() {
        for value in [
            "fd00:b00a::",
            "fd00:b00a::/129",
            "fd00:zz::/48",
            "172.30.0.0/16",
        ] {
            assert!(
                Cli::try_parse_from(["sherpad", "init", "--management-prefix-ipv6", value])
                    .is_err(),
                "{value} was accepted"
            );
        }
    }

    #[test]
    fn init_rejects_an_invalid_management_prefix() {
        for value in ["172.30.0.0", "172.30.0.0/33", "172.30.0/16", "fd00::/48"] {
            assert!(
                Cli::try_parse_from(["sherpad", "init", "--management-prefix-ipv4", value])
                    .is_err(),
                "{value} was accepted"
            );
        }
    }
}
