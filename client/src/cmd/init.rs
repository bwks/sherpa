use anyhow::Result;
use data::{Config, Sherpa};
use konst::{
    HTTP_PORT, SHERPA_ISOLATED_NETWORK_BRIDGE, SHERPA_ISOLATED_NETWORK_NAME,
    SHERPA_MANAGEMENT_NETWORK_BRIDGE, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_MANIFEST_FILE,
    SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_SSH_PUBLIC_KEY_FILE, SHERPA_STORAGE_POOL,
    SHERPA_STORAGE_POOL_PATH, TFTP_PORT,
};
use libvirt::{IsolatedNetwork, NatNetwork, Qemu, SherpaStoragePool};
use ssh_key::Algorithm;
use topology::Manifest;
use util::{
    create_dir, dir_exists, file_exists, generate_ssh_keypair, term_msg_highlight,
    term_msg_surround, term_msg_underline,
};

pub fn init(
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
        Config::load(&sherpa.config_path)?
    } else {
        create_dir(&sherpa.config_dir)?;
        create_dir(&format!("{}", sherpa.containers_dir,))?;
        create_dir(&format!("{}", sherpa.bins_dir,))?;
        create_dir(&sherpa.boxes_dir)?;
        // box directories
        let config = Config::default();
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
        let config = Config {
            name: config_file.to_owned(),
            ..Default::default()
        };
        config.create(&sherpa.config_path)?;
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
        .any(|net| net == SHERPA_MANAGEMENT_NETWORK_NAME)
    {
        println!("Network already exists: {SHERPA_MANAGEMENT_NETWORK_NAME}");
    } else {
        println!("Creating network: {SHERPA_MANAGEMENT_NETWORK_NAME}");
        let ipv4_network_size = config.management_prefix_ipv4.size();
        let management_network = NatNetwork {
            network_name: SHERPA_MANAGEMENT_NETWORK_NAME.to_owned(),
            bridge_name: SHERPA_MANAGEMENT_NETWORK_BRIDGE.to_owned(),
            ipv4_address: config.management_prefix_ipv4.nth(1).unwrap(),
            ipv4_netmask: config.management_prefix_ipv4.mask(),
            ipv4_default_gateway: config.management_prefix_ipv4.nth(1).unwrap(),
            dhcp_start: config.management_prefix_ipv4.nth(5).unwrap(),
            dhcp_end: config
                .management_prefix_ipv4
                .nth(ipv4_network_size - 2)
                .unwrap(),
            ztp_http_port: HTTP_PORT,
            ztp_tftp_port: TFTP_PORT,
            ztp_server_ipv4: config.management_prefix_ipv4.nth(5).unwrap(),
        };
        management_network.create(&qemu_conn)?;
    }

    // Create the isolated network
    if qemu_conn
        .list_networks()?
        .iter()
        .any(|net| net == SHERPA_ISOLATED_NETWORK_NAME)
    {
        println!("Network already exists: {SHERPA_ISOLATED_NETWORK_NAME}");
    } else {
        println!("Creating network: {SHERPA_ISOLATED_NETWORK_NAME}");
        let isolated_network = IsolatedNetwork {
            network_name: SHERPA_ISOLATED_NETWORK_NAME.to_owned(),
            bridge_name: SHERPA_ISOLATED_NETWORK_BRIDGE.to_owned(),
        };
        isolated_network.create(&qemu_conn)?;
    }
    let storage_pool = SherpaStoragePool {
        name: SHERPA_STORAGE_POOL.to_owned(),
        path: SHERPA_STORAGE_POOL_PATH.to_owned(),
    };
    storage_pool.create(&qemu_conn)?;
    Ok(())
}
