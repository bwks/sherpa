use anyhow::{Context, Result};
use db::{apply_schema, connect, upsert_user};
use libvirt::{BridgeNetwork, Qemu, SherpaStoragePool};
use shared::data::{NodeConfig, NodeModel, Sherpa};
use shared::konst::{
    SHERPA_BLANK_DISK_DIR, SHERPA_BRIDGE_NETWORK_BRIDGE, SHERPA_BRIDGE_NETWORK_NAME,
    SHERPA_DB_NAME, SHERPA_DB_NAMESPACE, SHERPA_DB_PORT, SHERPA_DB_SERVER, SHERPA_MANIFEST_FILE,
    SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_SSH_PUBLIC_KEY_FILE, SHERPA_STORAGE_POOL,
    SHERPA_STORAGE_POOL_PATH,
};
use shared::util::{
    create_config, create_dir, default_config, file_exists, find_user_ssh_keys,
    generate_ssh_keypair, get_username, term_msg_highlight, term_msg_surround, term_msg_underline,
};
use ssh_key::Algorithm;
use topology::Manifest;

pub async fn init(
    sherpa: &Sherpa,
    qemu: &Qemu,
    config_file: &str,
    manifest_file: &str,
    force: bool,
) -> Result<()> {
    term_msg_surround("Sherpa Initializing");
    let qemu_conn = qemu.connect()?;
    let sherpa = sherpa.clone();

    term_msg_highlight("Creating Files");

    create_dir(&sherpa.base_dir)?;
    create_dir(&sherpa.config_dir)?;
    create_dir(&sherpa.ssh_dir)?;
    create_dir(&sherpa.containers_dir.to_string())?;
    create_dir(&sherpa.bins_dir.to_string())?;
    create_dir(&sherpa.images_dir)?;
    create_dir(&format!("{}/{}", sherpa.images_dir, SHERPA_BLANK_DISK_DIR))?;

    // Initialize default config for container images
    let config = default_config();

    for container_image in &config.container_images {
        create_dir(&format!(
            "{}/{}",
            sherpa.containers_dir, container_image.name
        ))?;
    }

    // Initialize default files
    if file_exists(&sherpa.config_file_path) && !force {
        println!("Config file already exists: {}", sherpa.config_file_path);
    } else {
        let mut config = default_config();
        config.name = config_file.to_owned();
        create_config(&config, &sherpa.config_file_path)?;
    }

    if file_exists(manifest_file) && !force {
        println!("Manifest file already exists: {manifest_file}");
    } else {
        let manifest = Manifest::example()?;
        manifest.write_file(SHERPA_MANIFEST_FILE)?;
    }

    // SSH Keys
    let ssh_pub_key_file = format!("{}/{}", &sherpa.ssh_dir, SHERPA_SSH_PUBLIC_KEY_FILE);

    if file_exists(&ssh_pub_key_file) && !force {
        println!("SSH keys already exists: {ssh_pub_key_file}");
    } else {
        term_msg_underline("Creating SSH Keypair");
        generate_ssh_keypair(
            &sherpa.ssh_dir,
            SHERPA_SSH_PRIVATE_KEY_FILE,
            Algorithm::Rsa { hash: None }, // An RSA256 key will be generated.
        )?;
    }

    term_msg_highlight("Creating Networks");
    // Initialize the sherpa boot network
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

    // Create image directories from in-memory model definitions
    term_msg_underline("Creating Node Image Directories");
    let mut created_models = std::collections::HashSet::new();
    for model in NodeModel::to_vec() {
        let node_image = NodeConfig::get_model(model);
        if created_models.insert(node_image.model) {
            let model_dir = format!("{}/{}", sherpa.images_dir, node_image.model);
            create_dir(&model_dir)?;
        }
    }

    // Create database user for current system user
    term_msg_underline("Creating Database User");

    let username =
        get_username().context("Failed to detect current username for database user creation")?;

    let ssh_keys = find_user_ssh_keys();

    // TODO: This init command is being deprecated. User creation will be handled
    // by admin users via 'sherpa user create' command with proper password input.
    // For now, use a placeholder password that must be changed.
    let temp_password = "ChangeMe123!";

    upsert_user(
        &db,
        username.clone(),
        temp_password,
        false,
        ssh_keys.clone(),
    )
    .await
    .context(format!("Failed to create database user '{}'", username))?;

    if ssh_keys.is_empty() {
        println!("Created database user: {} (no SSH keys found)", username);
    } else {
        println!(
            "Created database user: {} ({} SSH key{} added)",
            username,
            ssh_keys.len(),
            if ssh_keys.len() == 1 { "" } else { "s" }
        );
    }

    Ok(())
}
