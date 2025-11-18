use anyhow::Result;
use container::Docker;
use data::Sherpa;
use konst::{
    SHERPA_BRIDGE_NETWORK_BRIDGE, SHERPA_BRIDGE_NETWORK_NAME, SHERPA_MANAGEMENT_NETWORK_NAME,
    SHERPA_MANIFEST_FILE, SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_SSH_PUBLIC_KEY_FILE,
    SHERPA_STORAGE_POOL, SHERPA_STORAGE_POOL_PATH,
};
use libvirt::{BridgeNetwork, Qemu, SherpaStoragePool};
use ssh_key::Algorithm;
use topology::Manifest;
use util::{
    create_config, create_dir, default_config, dir_exists, file_exists, generate_ssh_keypair,
    load_config, term_msg_highlight, term_msg_surround, term_msg_underline,
};

pub async fn init(
    sherpa: &Sherpa,
    qemu: &Qemu,
    config_file: &str,
    manifest_file: &str,
    force: bool,
) -> Result<()> {
    term_msg_surround("Sherpa Initializing");
    let qemu_conn = qemu.connect()?;

    let mut sherpa = sherpa.clone();

    sherpa.config_path = format!("{}/{}", sherpa.config_dir, config_file);

    term_msg_highlight("Creating Files");
    // Create the default config directories
    let config = if dir_exists(&sherpa.config_dir) && !force {
        println!("Directory path already exists: {}", sherpa.config_dir);
        load_config(&sherpa.config_path)?
    } else {
        create_dir(&sherpa.config_dir)?;
        create_dir(&format!("{}", sherpa.containers_dir,))?;
        create_dir(&format!("{}", sherpa.bins_dir,))?;
        create_dir(&sherpa.boxes_dir)?;
        // box directories
        let config = default_config();
        for device_model in &config.device_models {
            create_dir(&format!(
                "{}/{}/latest",
                sherpa.boxes_dir, device_model.name
            ))?;
        }

        for container_image in &config.container_images {
            create_dir(&format!(
                "{}/{}/latest",
                sherpa.containers_dir, container_image.name
            ))?;
        }
        config
    };

    // Initialize default files
    if file_exists(&sherpa.config_path) && !force {
        println!("Config file already exists: {}", sherpa.config_path);
    } else {
        let mut config = default_config();
        config.name = config_file.to_owned();
        create_config(&config, &sherpa.config_path)?;
    }

    if file_exists(manifest_file) && !force {
        println!("Manifest file already exists: {manifest_file}");
    } else {
        let manifest = Manifest::default()?;
        manifest.write_file(SHERPA_MANIFEST_FILE)?;
    }

    // SSH Keys
    let ssh_pub_key_file = format!("{}/{}", &sherpa.config_dir, SHERPA_SSH_PUBLIC_KEY_FILE);

    if file_exists(&ssh_pub_key_file) && !force {
        println!("SSH keys already exists: {ssh_pub_key_file}");
    } else {
        term_msg_underline("Creating SSH Keypair");
        generate_ssh_keypair(
            &sherpa.config_dir,
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
        println!("Network already exists: {SHERPA_MANAGEMENT_NETWORK_NAME}");
    } else {
        println!("Creating network: {SHERPA_MANAGEMENT_NETWORK_NAME}");
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
    Ok(())
}
