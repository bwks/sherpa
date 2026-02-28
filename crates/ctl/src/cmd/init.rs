use std::io::{self, Write};

use anyhow::{Context, Result};

use db::{apply_schema, connect, upsert_user};
use libvirt::{BridgeNetwork, Qemu, SherpaStoragePool};
use shared::data::{NodeConfig, NodeModel};
use shared::konst::{
    SHERPA_BLANK_DISK_DIR, SHERPA_BRIDGE_NETWORK_BRIDGE, SHERPA_BRIDGE_NETWORK_NAME,
    SHERPA_DB_NAME, SHERPA_DB_NAMESPACE, SHERPA_DB_PORT, SHERPA_DB_SERVER,
    SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_SSH_PUBLIC_KEY_FILE, SHERPA_STORAGE_POOL,
    SHERPA_STORAGE_POOL_PATH,
};
use shared::util::{
    create_config, create_dir, default_config, file_exists, generate_ssh_keypair,
    term_msg_highlight, term_msg_surround, term_msg_underline,
};
use ssh_key::Algorithm;

const SERVER_BASE_DIR: &str = "/opt/sherpa";
const SERVER_CONFIG_DIR: &str = "/opt/sherpa/config";
const SERVER_CONFIG_FILE: &str = "/opt/sherpa/config/sherpa.toml";
const SERVER_SSH_DIR: &str = "/opt/sherpa/ssh";
const SERVER_IMAGES_DIR: &str = "/opt/sherpa/images";
const SERVER_CONTAINERS_DIR: &str = "/opt/sherpa/containers";
const SERVER_BINS_DIR: &str = "/opt/sherpa/bins";

pub async fn init(force: bool) -> Result<()> {
    term_msg_surround("Sherpa Server Initializing");

    // Create server directories
    term_msg_highlight("Creating Directories");
    create_dir(SERVER_BASE_DIR)?;
    create_dir(SERVER_CONFIG_DIR)?;
    create_dir(SERVER_SSH_DIR)?;
    create_dir(SERVER_CONTAINERS_DIR)?;
    create_dir(SERVER_BINS_DIR)?;
    create_dir(SERVER_IMAGES_DIR)?;
    create_dir(&format!("{SERVER_IMAGES_DIR}/{SHERPA_BLANK_DISK_DIR}"))?;

    // Create image subdirectories for each node model
    term_msg_underline("Creating Node Image Directories");
    let mut created_models = std::collections::HashSet::new();
    for model in NodeModel::to_vec() {
        let node_image = NodeConfig::get_model(model);
        if created_models.insert(node_image.model) {
            let model_dir = format!("{SERVER_IMAGES_DIR}/{}", node_image.model);
            create_dir(&model_dir)?;
        }
    }

    // Create container image directories from default config
    let config = default_config();
    for container_image in &config.container_images {
        create_dir(&format!("{SERVER_CONTAINERS_DIR}/{}", container_image.name))?;
    }

    // Write server config
    if file_exists(SERVER_CONFIG_FILE) && !force {
        println!("Config file already exists: {SERVER_CONFIG_FILE}");
    } else {
        term_msg_underline("Writing Server Config");
        let config = default_config();
        create_config(&config, SERVER_CONFIG_FILE)?;
        println!("Config written to: {SERVER_CONFIG_FILE}");
    }

    // SSH Keys
    let ssh_pub_key_file = format!("{SERVER_SSH_DIR}/{SHERPA_SSH_PUBLIC_KEY_FILE}");
    if file_exists(&ssh_pub_key_file) && !force {
        println!("SSH keys already exist: {ssh_pub_key_file}");
    } else {
        term_msg_underline("Creating SSH Keypair");
        generate_ssh_keypair(
            SERVER_SSH_DIR,
            SHERPA_SSH_PRIVATE_KEY_FILE,
            Algorithm::Rsa { hash: None },
        )?;
    }

    // Libvirt network and storage pool
    term_msg_highlight("Creating Networks");
    let qemu = Qemu::default();
    let qemu_conn = qemu.connect()?;

    if qemu_conn
        .list_networks()?
        .iter()
        .any(|net| net == SHERPA_BRIDGE_NETWORK_NAME)
    {
        println!("Network already exists: {SHERPA_BRIDGE_NETWORK_NAME}");
    } else {
        println!("Creating network: {SHERPA_BRIDGE_NETWORK_NAME}");
        let bridge_network = BridgeNetwork {
            network_name: SHERPA_BRIDGE_NETWORK_NAME.to_owned(),
            bridge_name: SHERPA_BRIDGE_NETWORK_BRIDGE.to_owned(),
        };
        bridge_network.create(&qemu_conn)?;
    }

    let storage_pool = SherpaStoragePool {
        name: SHERPA_STORAGE_POOL.to_owned(),
        path: SHERPA_STORAGE_POOL_PATH.to_owned(),
    };
    storage_pool.create(&qemu_conn)?;

    // Database initialization
    term_msg_highlight("Initializing Database");
    let db = connect(
        SHERPA_DB_SERVER,
        SHERPA_DB_PORT,
        SHERPA_DB_NAMESPACE,
        SHERPA_DB_NAME,
    )
    .await?;

    term_msg_underline("Applying Database Schema");
    apply_schema(&db).await?;
    println!("Database schema applied");

    // Create initial admin user
    term_msg_underline("Creating Admin User");
    let username = prompt("Admin username")?;
    let password = prompt_password(&username)?;

    upsert_user(&db, username.clone(), &password, true, vec![])
        .await
        .context(format!("Failed to create admin user '{username}'"))?;

    println!("Admin user '{}' created", username);

    term_msg_surround("Sherpa Server Initialized");
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
