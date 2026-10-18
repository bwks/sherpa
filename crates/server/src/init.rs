use std::env;
use std::fs;
use std::io::{self, Write};
use std::net::Ipv4Addr;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

use db::{Database, apply_schema, connect, create_admin_user, create_user, find_user, list_users};
use libvirt::{BridgeNetwork, Qemu, SherpaStoragePool};
use shared::data::{NodeConfig, NodeModel};
use shared::konst::{
    ADMIN_USERNAME, SHERPA_ADMIN_PASSWORD_ENV, SHERPA_BASE_DIR, SHERPA_BINS_PATH,
    SHERPA_BLANK_DISK_DIR, SHERPA_BRIDGE_NETWORK_BRIDGE, SHERPA_BRIDGE_NETWORK_NAME,
    SHERPA_CONFIG_FILE_PATH, SHERPA_CONFIG_PATH, SHERPA_CONTAINERS_PATH, SHERPA_DB_NAME,
    SHERPA_DB_NAMESPACE, SHERPA_DB_PORT, SHERPA_DB_SERVER, SHERPA_ENV_FILE_PATH,
    SHERPA_IMAGES_PATH, SHERPA_SERVER_HTTP_PORT, SHERPA_SERVER_IPV4, SHERPA_SERVER_WS_PORT,
    SHERPA_SSH_PATH, SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_SSH_PUBLIC_KEY_PATH, SHERPA_STORAGE_POOL,
    SHERPA_STORAGE_POOL_PATH,
};
use shared::util::{
    create_config, create_dir, default_config, file_exists, generate_ssh_keypair,
    normalize_username, read_env_file_value, term_msg_highlight, term_msg_surround,
    term_msg_underline,
};
use shared::validators::{validate_ssh_key, validate_username};
use ssh_key::Algorithm;

pub async fn init(
    force: bool,
    db_password: Option<&str>,
    server_ipv4: Option<&str>,
    ws_port: Option<u16>,
    http_port: Option<u16>,
    db_port: Option<u16>,
) -> Result<()> {
    let env_file = Path::new(SHERPA_ENV_FILE_PATH);

    let db_password = match db_password {
        Some(p) => p.to_string(),
        None => read_env_file_value(env_file, "SHERPA_DB_PASSWORD").ok_or_else(|| {
            anyhow::anyhow!(
                "Database password not provided. Supply it via:\n  \
                     1. --db-pass flag\n  \
                     2. SHERPA_DB_PASSWORD environment variable\n  \
                     3. SHERPA_DB_PASSWORD entry in {}",
                env_file.display()
            )
        })?,
    };

    let server_ipv4 = match server_ipv4 {
        Some(ip) => ip.to_string(),
        None => read_env_file_value(env_file, "SHERPA_SERVER_IPV4")
            .unwrap_or_else(|| SHERPA_SERVER_IPV4.to_string()),
    };

    let ws_port = match ws_port {
        Some(p) => p,
        None => read_env_file_value(env_file, "SHERPA_SERVER_WS_PORT")
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(SHERPA_SERVER_WS_PORT),
    };

    let http_port = match http_port {
        Some(p) => p,
        None => read_env_file_value(env_file, "SHERPA_SERVER_HTTP_PORT")
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(SHERPA_SERVER_HTTP_PORT),
    };

    let db_port = match db_port {
        Some(p) => p,
        None => read_env_file_value(env_file, "SHERPA_DB_PORT")
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(SHERPA_DB_PORT),
    };

    let server_ipv4_addr: Ipv4Addr = server_ipv4
        .parse()
        .context("Invalid server IP address. Expected format: x.x.x.x")?;

    let server_ipv6_addr: Option<std::net::Ipv6Addr> =
        read_env_file_value(env_file, "SHERPA_SERVER_IPV6").and_then(|v| v.parse().ok());

    term_msg_surround("Sherpa Server Initializing");

    // Create server directories
    term_msg_highlight("Creating Directories");
    create_dir(SHERPA_BASE_DIR)?;
    create_dir(SHERPA_CONFIG_PATH)?;
    create_dir(SHERPA_SSH_PATH)?;
    create_dir(SHERPA_CONTAINERS_PATH)?;
    create_dir(SHERPA_BINS_PATH)?;
    create_dir(SHERPA_IMAGES_PATH)?;
    create_dir(&format!("{SHERPA_IMAGES_PATH}/{SHERPA_BLANK_DISK_DIR}"))?;

    // Create image subdirectories for each node model
    term_msg_underline("Creating Node Image Directories");
    let mut created_models = std::collections::HashSet::new();
    for model in NodeModel::to_vec() {
        let node_image = NodeConfig::get_model(model);
        if created_models.insert(node_image.model) {
            let model_dir = format!("{SHERPA_IMAGES_PATH}/{}", node_image.model);
            create_dir(&model_dir)?;
        }
    }

    // Create container image directories from default config
    let config = default_config();
    for container_image in &config.container_images {
        create_dir(&format!(
            "{SHERPA_CONTAINERS_PATH}/{}",
            container_image.name
        ))?;
    }

    // Write server config
    if file_exists(SHERPA_CONFIG_FILE_PATH) && !force {
        println!("Config file already exists: {SHERPA_CONFIG_FILE_PATH}");
    } else {
        term_msg_underline("Writing Server Config");
        let mut config = default_config();
        config.server_ipv4 = server_ipv4_addr;
        config.server_ipv6 = server_ipv6_addr;
        config.ws_port = ws_port;
        config.http_port = http_port;
        create_config(&config, SHERPA_CONFIG_FILE_PATH)?;
        println!("Config written to: {SHERPA_CONFIG_FILE_PATH}");
    }

    // SSH Keys
    if file_exists(SHERPA_SSH_PUBLIC_KEY_PATH) && !force {
        println!("SSH keys already exist: {SHERPA_SSH_PUBLIC_KEY_PATH}");
    } else {
        term_msg_underline("Creating SSH Keypair");
        generate_ssh_keypair(
            SHERPA_SSH_PATH,
            SHERPA_SSH_PRIVATE_KEY_FILE,
            Algorithm::Rsa { hash: None },
        )?;
    }

    // Libvirt network and storage pool
    let qemu = Qemu::default();
    let qemu_conn = qemu.connect()?;

    term_msg_highlight("Creating Networks");
    println!("Creating network: {SHERPA_BRIDGE_NETWORK_NAME}");
    let bridge_network = BridgeNetwork {
        network_name: SHERPA_BRIDGE_NETWORK_NAME.to_owned(),
        bridge_name: SHERPA_BRIDGE_NETWORK_BRIDGE.to_owned(),
    };
    bridge_network.create(&qemu_conn)?;

    term_msg_highlight("Creating Storage Pools");
    println!("Creating storage pool: {SHERPA_STORAGE_POOL}");
    let storage_pool = SherpaStoragePool {
        name: SHERPA_STORAGE_POOL.to_owned(),
        path: SHERPA_STORAGE_POOL_PATH.to_owned(),
    };
    storage_pool.create(&qemu_conn)?;

    // Database initialization
    term_msg_highlight("Initializing Database");
    let db = connect(
        SHERPA_DB_SERVER,
        db_port,
        SHERPA_DB_NAMESPACE,
        SHERPA_DB_NAME,
        &db_password,
    )
    .await?;

    term_msg_underline("Applying Database Schema");
    apply_schema(&db).await?;
    println!("Database schema applied");

    create_admin_account(&db, env_file).await?;
    create_first_user(&db).await?;

    term_msg_surround("Sherpa Server Initialized");
    Ok(())
}

/// Create the built-in `admin` account unless it already exists.
///
/// The password comes from `SHERPA_ADMIN_PASSWORD` in the environment or the env file, or is
/// prompted for. An existing `admin` is left untouched, so `init` is safe to re-run.
#[instrument(skip(db, env_file))]
async fn create_admin_account(db: &Database, env_file: &Path) -> Result<()> {
    term_msg_underline("Creating Admin Account");
    if find_user(db, ADMIN_USERNAME)
        .await
        .context("Failed to look up the admin account")?
        .is_some()
    {
        println!("Admin account '{ADMIN_USERNAME}' already exists");
        return Ok(());
    }

    let password = match env::var(SHERPA_ADMIN_PASSWORD_ENV)
        .ok()
        .or_else(|| read_env_file_value(env_file, SHERPA_ADMIN_PASSWORD_ENV))
    {
        Some(password) => {
            println!("Using the admin password from {SHERPA_ADMIN_PASSWORD_ENV}");
            password
        }
        None => prompt_password(ADMIN_USERNAME)?,
    };
    create_admin_user(db, &password)
        .await
        .context("Failed to create the admin account")?;
    println!("Admin account '{ADMIN_USERNAME}' created");
    Ok(())
}

/// Create the first personal user account, unless one already exists.
///
/// Lab work is done from personal accounts; `admin` cannot start labs.
#[instrument(skip(db))]
async fn create_first_user(db: &Database) -> Result<()> {
    term_msg_underline("Creating First User Account");
    let users = list_users(db)
        .await
        .context("Failed to list user accounts")?;
    if users.iter().any(|user| user.username != ADMIN_USERNAME) {
        println!("User accounts already exist, skipping");
        return Ok(());
    }

    println!("Labs are started from personal user accounts, not from '{ADMIN_USERNAME}'.");
    let username = normalize_username(&prompt("Username")?);
    validate_username(&username)?;
    let password = prompt_password(&username)?;
    let is_admin = prompt_yes_no("Make this user an administrator?", true)?;
    let ssh_keys = prompt_ssh_key(&username)?;

    if ssh_keys.is_empty() {
        println!(
            "No SSH key added. '{username}' needs one to start labs. Add it on the web UI Profile \
             page or with 'sherpa server user ssh-key add'."
        );
    }
    create_user(db, username.clone(), &password, is_admin, ssh_keys)
        .await
        .context(format!("Failed to create user '{username}'"))?;
    println!("User '{username}' created");
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{}: ", label);
    io::stdout().flush().context("Failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("Failed to read input")?;

    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("{label} cannot be empty");
    }
    Ok(trimmed)
}

fn prompt_optional(label: &str) -> Result<String> {
    print!("{}: ", label);
    io::stdout().flush().context("Failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("Failed to read input")?;
    Ok(input.trim().to_string())
}

fn prompt_yes_no(question: &str, default: bool) -> Result<bool> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    let answer = prompt_optional(&format!("{question} {hint}"))?;
    match answer.to_lowercase().as_str() {
        "" => Ok(default),
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        other => anyhow::bail!("Expected yes or no, got '{other}'"),
    }
}

/// Ask for an SSH public key as a file path or the key itself. Enter skips it.
fn prompt_ssh_key(username: &str) -> Result<Vec<String>> {
    let answer = prompt_optional(&format!(
        "SSH public key for {username} (file path or key, Enter to skip)"
    ))?;
    if answer.is_empty() {
        return Ok(vec![]);
    }
    let key = if Path::new(&answer).is_file() {
        fs::read_to_string(&answer)
            .context(format!("Failed to read SSH public key file '{answer}'"))?
            .trim()
            .to_string()
    } else {
        answer
    };
    validate_ssh_key(&key).context("Invalid SSH public key")?;
    Ok(vec![key])
}

fn prompt_password(username: &str) -> Result<String> {
    let password = rpassword::prompt_password(format!("Password for {username}: "))
        .context("Failed to read password")?;
    let confirm =
        rpassword::prompt_password("Confirm password: ").context("Failed to read password")?;

    if password != confirm {
        anyhow::bail!("Passwords do not match");
    }
    if password.is_empty() {
        anyhow::bail!("Password cannot be empty");
    }
    Ok(password)
}
