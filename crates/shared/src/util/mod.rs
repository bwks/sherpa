mod config;
mod dns;
mod emoji;
mod encode;
mod env;
mod file_system;
mod host;
mod interface;
mod ip;
mod lab_folder;
mod lab_name;
mod mac;
mod output;
mod random;
mod sanitizers;
mod ssh;
mod table;
mod text;
mod user;

pub use config::{
    build_client_websocket_url, create_client_config, create_config, default_config,
    load_client_config, load_config,
};
pub use dns::{default_dns, default_dns_dual_stack};
pub use emoji::{Emoji, emoji_success, emoji_warning};
pub use encode::{base64_decode, base64_decode_bytes, base64_encode, base64_encode_bytes};
pub use env::get_server_url;
pub use file_system::{
    copy_file, create_dir, create_file, delete_dirs, dir_exists, expand_path, file_exists, get_cwd,
    image_filename, load_file, path_to_string,
};
#[cfg(unix)]
pub use file_system::{
    copy_to_dos_image, copy_to_ext4_image, create_config_archive, fix_permissions_recursive,
    set_file_permissions,
};
pub use host::{get_fqdn, get_hostname};
#[cfg(feature = "netinfo")]
pub use host::{get_non_loopback_ipv4_addresses, get_non_loopback_ipv6_addresses};
pub use interface::{
    container_interface_name, interface_from_idx, interface_to_idx, node_model_interfaces,
    srlinux_to_linux_interface,
};
pub use ip::{
    allocate_ipv6_loopback_subnet, allocate_ipv6_management_subnet, allocate_loopback_subnet,
    allocate_management_subnet, get_ip, get_ipv4_addr, get_ipv4_network, get_ipv6_addr,
    get_ipv6_network, ipv4_overlaps_any, ipv6_overlaps_any, node_management_host_offset,
};
#[cfg(feature = "netinfo")]
pub use ip::{get_interface_networks, get_interface_networks_v6};
pub use lab_folder::{
    LabFolder, keycloak_file_name, kubeconfig_file_name, talosconfig_file_name, vault_file_name,
};
pub use lab_name::is_lab_resource;
pub use mac::random_mac;
pub use output::{
    confirm, display_destroy_results, term_msg_highlight, term_msg_surround, term_msg_underline,
};
pub use random::{generate_lab_name, get_id, get_id_for_user};
pub use sanitizers::dasher;
pub use ssh::{
    SshConfigCleanReport, SshConfigEntryStatus, SshConfigInspectionEntry,
    SshConfigInspectionReport, add_lab_ssh_include, clean_stale_lab_ssh_includes,
    generate_ssh_keypair, get_ssh_public_key, get_user_ssh_dir, inspect_lab_ssh_includes,
    pub_ssh_key_to_md5_hash, pub_ssh_key_to_sha256_hash, remove_lab_ssh_include,
    remove_lab_ssh_include_at, replace_lab_ssh_include, ssh_key_fingerprint,
};
pub use table::{
    CertificateTableInfo, render_backup_table, render_bridges_table, render_certificates_table,
    render_devices_table, render_image_detail_table, render_images_table, render_lab_info_table,
    render_links_table, render_nodes_table, render_provision_table, render_scanned_images_table,
    render_server_status_table, render_ssh_config_inspection_table, render_ssh_keys_table,
    render_tailnet_table,
};
pub use text::split_node_int;
pub use user::{get_username, normalize_username, sherpa_user};

mod manifest;
pub use manifest::load_saved_manifest;
