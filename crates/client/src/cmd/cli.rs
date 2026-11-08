use std::io::{self, ErrorKind};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use super::cert::{cert_delete, cert_list, cert_show, cert_trust};
use super::console::console;
use super::destroy::destroy;
use super::down::down;
use super::download::download;
use super::image::{ImageCommands, parse_image_commands};
use super::init::init;
use super::inspect::inspect;
use super::lab_identity::{manifest_lab_identity, resolve_lab_identity};
use super::link::{LinkCommands, parse_link_commands};
use super::login::{login, logout, whoami};
use super::new::new;
use super::provision::provision;
use super::redeploy::redeploy;
use super::resume::resume;
use super::server::{OutputFormat, ServerCommands, run_server};
use super::ssh::ssh;
use super::ssh_config::{ssh_config_clean, ssh_config_inspect};
use super::up::up;
use super::validate::validate_manifest;

use crate::lab_folder_migration::{MIGRATION_FAILED, migrate_current_lab_folder};
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

use shared::data::{ClientConfig, InspectResponse, LabInfo, Sherpa};
use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::{
    Emoji, LabFolder, build_client_websocket_url, get_cwd, get_server_url, load_client_config,
    load_file,
};
use topology::Manifest;

/// Load the client config from `path`, or the defaults when there is no such file. A file that
/// cannot be read or parsed is an error, rather than silently replaced by the defaults.
fn load_client_config_or_default(path: &str) -> Result<ClientConfig> {
    match load_client_config(path) {
        Ok(config) => Ok(config),
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
        {
            Ok(ClientConfig::default())
        }
        Err(error) => Err(error.context(format!("Failed to load the client config {path}"))),
    }
}

fn apply_insecure_override(config: &mut ClientConfig, insecure: bool) {
    if insecure {
        config.server_connection.insecure = true;
        eprintln!("WARNING: TLS certificate validation disabled (--insecure)");
    }
}

fn load_client_config_for_command(path: &str, insecure: bool) -> Result<ClientConfig> {
    let mut config = load_client_config_or_default(path)?;
    apply_insecure_override(&mut config, insecure);
    Ok(config)
}

/// Resolve the server URL from CLI args, env, or config.
fn resolve_server_url(cli_url: Option<String>, config: &ClientConfig) -> String {
    cli_url
        .or_else(get_server_url)
        .unwrap_or_else(|| build_client_websocket_url(config))
}

/// Resolve lab info: try `.sherpa/lab-info.toml` first, fall back to inspect RPC.
#[tracing::instrument(level = "debug", skip_all, fields(%lab_id))]
async fn resolve_lab_info(
    lab_id: &str,
    server_url: &str,
    config: &ClientConfig,
) -> Result<LabInfo> {
    // Fast path: try local .sherpa/lab-info.toml
    if let Ok(cwd) = get_cwd() {
        let lab_info_path = LabFolder::new(Path::new(&cwd)).lab_info();
        if lab_info_path.exists()
            && let Ok(content) = load_file(&lab_info_path.to_string_lossy())
            && let Ok(lab_info) = LabInfo::from_str(&content)
        {
            return Ok(lab_info);
        }
    }

    // Slow path: fetch from server via inspect RPC
    let token = load_token().context(
        ".sherpa/lab-info.toml not found and no auth token available. Run: sherpa login",
    )?;

    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    let mut rpc_client = ws_client
        .connect()
        .await
        .context(".sherpa/lab-info.toml not found and failed to connect to server")?;

    let request = RpcRequest::new(
        "inspect",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );

    let response = rpc_client
        .call(request)
        .await
        .context("RPC inspect call failed")?;
    rpc_client.close().await.ok();

    let inspect_data: InspectResponse =
        response.into_result("Failed to fetch lab info from server")?;

    Ok(inspect_data.lab_info)
}

/// Resolve the Sherpa client base directory (~/.sherpa)
fn sherpa_client_base_dir() -> String {
    dirs::home_dir().map_or_else(
        || ".sherpa".to_string(),
        |h| h.join(".sherpa").to_string_lossy().to_string(),
    )
}

#[derive(Default, Debug, Parser)]
#[command(name = "sherpa")]
#[command(bin_name = "sherpa")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Sherpa - Network Lab Management", long_about = None)]
pub struct Cli {
    /// Remote server URL for WebSocket RPC (e.g., <ws://localhost:3030/ws>)
    #[arg(long, global = true, env = "SHERPA_SERVER_URL")]
    server_url: Option<String>,

    /// Skip TLS certificate validation (insecure - for development only)
    #[arg(long, global = true)]
    insecure: bool,

    #[clap(subcommand)]
    commands: Commands,
}

#[derive(Default, Debug, Subcommand)]
enum Commands {
    /// Login to Sherpa server
    Login,
    /// Logout from Sherpa server
    Logout,
    /// Show current authentication status
    Whoami,

    /// Create a new example manifest.toml in the current directory
    New {
        /// Overwrite manifest file if one exists
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    /// Initialise a Sherpa client environment
    Init {
        /// Overwrite config file if one exists
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },
    /// Build environment
    Up,

    /// Stop environment
    Down {
        /// Target a specific node (omit for all nodes)
        #[arg(short, long)]
        node: Option<String>,
    },
    /// Resume environment
    Resume {
        /// Target a specific node (omit for all nodes)
        #[arg(short, long)]
        node: Option<String>,
    },
    /// Redeploy a single node (destroy and recreate with fresh ZTP)
    Redeploy {
        /// Target node to redeploy
        #[arg(short, long)]
        node: String,
    },
    /// Run node provisioners again (resend container secret files; copy files and run scripts over SSH)
    Provision {
        /// Target a specific node (omit for all nodes with provisioners)
        #[arg(short, long)]
        node: Option<String>,
    },
    /// Destroy environment
    Destroy {
        /// Skip confirmation prompt
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },
    /// Inspect environment
    #[default]
    Inspect,

    /// Download lab files (SSH config, SSH key, lab-info) for CLI use
    Download {
        /// Lab ID to download files for
        lab_id: String,
        /// Overwrite existing lab files if they exist
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        force: bool,
    },

    /// Validate configurations
    Validate,

    /// Connect to a device via serial console over Telnet
    Console { name: String },

    /// SSH to a device.
    Ssh { name: String },

    /// Inspect or clean Sherpa SSH config Include entries.
    SshConfig {
        #[command(subcommand)]
        command: SshConfigCommands,
    },

    /// Link commands
    Link {
        #[command(subcommand)]
        commands: LinkCommands,
    },

    /// Image management commands
    Image {
        #[command(subcommand)]
        commands: ImageCommands,
    },

    /// Certificate management commands (TOFU system)
    Cert {
        #[command(subcommand)]
        commands: CertCommands,
    },

    /// Server administration commands
    Server {
        /// Enable verbose logging
        #[arg(short, long)]
        verbose: bool,

        /// Output format (text or json)
        #[arg(long, default_value = "text")]
        output: OutputFormat,

        #[command(subcommand)]
        commands: ServerCommands,
    },
}

#[derive(Debug, Subcommand)]
enum SshConfigCommands {
    /// Inspect and validate Sherpa SSH config Include entries.
    Inspect,
    /// Remove stale or broken Sherpa SSH config Include entries.
    Clean,
}

#[derive(Debug, Subcommand)]
enum CertCommands {
    /// List all trusted certificates
    List,
    /// Show detailed certificate information
    Show {
        /// Server URL (e.g., <wss://10.100.58.10:3030/ws>)
        server_url: String,
    },
    /// Trust a certificate without connecting
    Trust {
        /// Server URL (e.g., <wss://10.100.58.10:3030/ws>)
        server_url: String,
    },
    /// Delete a trusted certificate
    Delete {
        /// Server URL (e.g., <wss://10.100.58.10:3030/ws>)
        server_url: String,
    },
}

impl Cli {
    /// Run the command line's command.
    ///
    /// # Errors
    ///
    /// Fails when the command fails, with a non-zero exit from `main`.
    pub async fn run() -> Result<()> {
        let cli = Cli::parse();
        let sherpa = Sherpa::from_base_dir(sherpa_client_base_dir());
        match &cli.commands {
            Commands::Logout => logout(),
            Commands::New { force } => new(*force),
            Commands::Init { force } => init(&sherpa, *force),
            Commands::Validate => validate_manifest(SHERPA_MANIFEST_FILE),
            Commands::Cert { commands } => run_cert(commands, &sherpa).await,
            command => {
                if uses_lab_folder(command) {
                    tolerate_failed_migration(command, migrate_current_lab_folder())?;
                }
                let config =
                    load_client_config_for_command(&sherpa.config_file_path, cli.insecure)?;
                let server_url = resolve_server_url(cli.server_url.clone(), &config);
                run_remote(command, &server_url, &config).await
            }
        }
    }
}

/// Whether `command` reads or writes the lab files in the current folder, so must move those
/// an older client left loose into `.sherpa` first (`lab_folder_migration`).
fn uses_lab_folder(command: &Commands) -> bool {
    !matches!(
        command,
        Commands::Login
            | Commands::Whoami
            | Commands::Logout
            | Commands::New { .. }
            | Commands::Init { .. }
            | Commands::Validate
            | Commands::Cert { .. }
            | Commands::Image { .. }
            | Commands::Server { .. }
    )
}

/// The result of migrating the lab folder for `command`, with a failure said to be the
/// migration's. A failed migration stops every command but `destroy`, which warns instead: the
/// lab must be destroyable whatever state its files are in, and its local cleanup handles the
/// old layout too.
///
/// # Errors
///
/// Fails with the migration's error unless `command` is `destroy`.
fn tolerate_failed_migration(command: &Commands, migration: Result<()>) -> Result<()> {
    match migration.context(MIGRATION_FAILED) {
        Err(error) if matches!(command, Commands::Destroy { .. }) => {
            tracing::warn!(error = %format!("{error:#}"), "Lab folder migration failed");
            eprintln!("{}", failed_migration_warning(&error));
            Ok(())
        }
        other => other,
    }
}

/// The warning `destroy` shows when the lab folder migration failed with `error`.
fn failed_migration_warning(error: &anyhow::Error) -> String {
    format!(
        "{} {error:#}\n   Continuing: destroy removes the lab's files either way",
        Emoji::Warning
    )
}

/// Run a trusted certificate command; these only read and change the local trust store.
async fn run_cert(command: &CertCommands, sherpa: &Sherpa) -> Result<()> {
    match command {
        CertCommands::List => cert_list(),
        CertCommands::Show { server_url } => cert_show(server_url),
        CertCommands::Trust { server_url } => {
            let config = load_client_config_or_default(&sherpa.config_file_path)?;
            cert_trust(server_url, config.server_connection.http_port).await
        }
        CertCommands::Delete { server_url } => cert_delete(server_url),
    }
}

/// Run a command that talks to the Sherpa server at `server_url`.
async fn run_remote(command: &Commands, server_url: &str, config: &ClientConfig) -> Result<()> {
    match command {
        Commands::Login => login(server_url, config).await,
        Commands::Whoami => whoami(server_url, config).await,
        Commands::Up => {
            let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
            let lab = manifest_lab_identity(&manifest)?;
            up(&lab.name, &lab.id, SHERPA_MANIFEST_FILE, server_url, config).await
        }
        Commands::Down { node } => {
            let lab = resolve_lab_identity()?;
            down(&lab.name, &lab.id, node.as_deref(), server_url, config).await
        }
        Commands::Resume { node } => {
            let lab = resolve_lab_identity()?;
            resume(&lab.name, &lab.id, node.as_deref(), server_url, config).await
        }
        Commands::Redeploy { node } => {
            let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
            let lab = manifest_lab_identity(&manifest)?;
            redeploy(
                &lab.name,
                &lab.id,
                node,
                SHERPA_MANIFEST_FILE,
                server_url,
                config,
            )
            .await
        }
        Commands::Provision { node } => provision(node.as_deref(), server_url, config).await,
        Commands::Destroy { yes } => {
            let lab = resolve_lab_identity()?;
            destroy(&lab.name, &lab.id, server_url, config, *yes).await
        }
        Commands::Inspect => {
            let lab = resolve_lab_identity()?;
            inspect(&lab.name, &lab.id, server_url, config).await
        }
        Commands::Download { lab_id, force } => download(lab_id, *force, server_url, config).await,
        Commands::Console { name } => {
            let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
            let lab_id = manifest_lab_identity(&manifest)?.id;
            let lab_info = resolve_lab_info(&lab_id, server_url, config).await?;
            console(name, &manifest, &lab_info)
        }
        Commands::Ssh { name } => {
            let lab = resolve_lab_identity()?;
            ssh(name, &lab.id, server_url, config).await
        }
        Commands::SshConfig { command } => match command {
            SshConfigCommands::Inspect => ssh_config_inspect(server_url, config).await,
            SshConfigCommands::Clean => ssh_config_clean(server_url, config).await,
        },
        Commands::Link { commands } => parse_link_commands(commands, config, server_url).await,
        Commands::Image { commands } => parse_image_commands(commands, config, server_url).await,
        Commands::Server {
            verbose,
            output,
            commands,
        } => run_server(commands, *verbose, output, server_url, config).await,
        Commands::Logout
        | Commands::New { .. }
        | Commands::Init { .. }
        | Commands::Validate
        | Commands::Cert { .. } => bail!("{command:?} runs without the server"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::lab_folder_migration::migrate_lab_folder;

    #[test]
    fn a_missing_client_config_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sherpa.toml");
        let config = load_client_config_or_default(path.to_str().unwrap()).unwrap();
        assert_eq!(config.server_ipv4, ClientConfig::default().server_ipv4);
    }

    #[test]
    fn a_client_config_that_does_not_parse_is_an_error_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sherpa.toml");
        fs::write(&path, "server_ipv4 = [").unwrap();
        let error = load_client_config_or_default(path.to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("sherpa.toml"), "{error}");
    }

    #[test]
    fn test_apply_insecure_override_sets_config_flag() {
        let mut config = ClientConfig::default();

        apply_insecure_override(&mut config, true);

        assert!(config.server_connection.insecure);
    }

    #[test]
    fn test_apply_insecure_override_leaves_config_secure_by_default() {
        let mut config = ClientConfig::default();

        apply_insecure_override(&mut config, false);

        assert!(!config.server_connection.insecure);
    }

    #[test]
    fn test_parse_ssh_node_command_unchanged() {
        let cli = Cli::try_parse_from(["sherpa", "ssh", "leaf1"]).unwrap();
        match cli.commands {
            Commands::Ssh { name } => assert_eq!(name, "leaf1"),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn only_destroy_goes_on_after_a_failed_migration() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("sherpa").chain(args.iter().copied()))
                .unwrap()
                .commands
        };
        let failure = || Err(anyhow::anyhow!("conflict"));

        assert!(tolerate_failed_migration(&parse(&["destroy", "--yes"]), failure()).is_ok());
        for args in [&["ssh", "dev"][..], &["up"], &["inspect"], &["down"]] {
            let error = tolerate_failed_migration(&parse(args), failure()).unwrap_err();
            assert_eq!(
                format!("{error:#}"),
                "Failed to move the lab files into .sherpa: conflict",
                "{args:?}"
            );
        }
        assert!(tolerate_failed_migration(&parse(&["ssh", "dev"]), Ok(())).is_ok());
    }

    #[test]
    fn a_failed_migration_says_it_was_the_migration_once() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let state_dir = lab.path().join(".sherpa");
        fs::write(&state_dir, "").unwrap();
        let migration = || migrate_lab_folder(lab.path(), &home.path().join(".ssh")).map(|_| ());
        let up = Cli::try_parse_from(["sherpa", "up"]).unwrap().commands;
        let expected = format!(
            "Failed to move the lab files into .sherpa: {} is not a folder",
            state_dir.display()
        );

        let error = tolerate_failed_migration(&up, migration()).unwrap_err();
        assert_eq!(format!("{error:#}"), expected);

        let error = migration().context(MIGRATION_FAILED).unwrap_err();
        let warning = failed_migration_warning(&error);
        assert!(warning.contains(&expected), "{warning}");
        assert_eq!(warning.matches(MIGRATION_FAILED).count(), 1, "{warning}");
    }

    #[test]
    fn commands_that_use_the_lab_folder_migrate_it_first() {
        let uses = |args: &[&str]| {
            let cli =
                Cli::try_parse_from(std::iter::once("sherpa").chain(args.iter().copied())).unwrap();
            uses_lab_folder(&cli.commands)
        };
        for args in [
            &["up"][..],
            &["down"],
            &["resume"],
            &["provision"],
            &["destroy", "--yes"],
            &["inspect"],
            &["download", "abcd1234"],
            &["ssh", "dev"],
            &["console", "dev"],
            &["ssh-config", "inspect"],
            &["ssh-config", "clean"],
        ] {
            assert!(uses(args), "{args:?}");
        }
        for args in [&["login"][..], &["whoami"], &["logout"], &["validate"]] {
            assert!(!uses(args), "{args:?}");
        }
    }

    #[test]
    fn test_parse_ssh_config_inspect_subcommand() {
        let cli = Cli::try_parse_from(["sherpa", "ssh-config", "inspect"]).unwrap();
        match cli.commands {
            Commands::SshConfig {
                command: SshConfigCommands::Inspect,
            } => {}
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn at_most_one_image_kind_can_be_given() {
        for args in [
            &["sherpa", "image", "list", "--container", "--unikernel"][..],
            &[
                "sherpa",
                "server",
                "image",
                "list",
                "--unikernel",
                "--virtual-machine",
            ],
            &[
                "sherpa",
                "server",
                "image",
                "scan",
                "--container",
                "--virtual-machine",
            ],
        ] {
            assert!(Cli::try_parse_from(args).is_err(), "{args:?}");
        }
        assert!(Cli::try_parse_from(["sherpa", "server", "image", "scan", "--container"]).is_ok());
    }

    #[test]
    fn test_parse_ssh_config_clean_subcommand() {
        let cli = Cli::try_parse_from(["sherpa", "ssh-config", "clean"]).unwrap();
        match cli.commands {
            Commands::SshConfig {
                command: SshConfigCommands::Clean,
            } => {}
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
