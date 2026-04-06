mod config;
mod dhcp;
mod dns;
mod emoji;
mod encode;
mod env;
mod file_system;
mod host;
mod interface;
mod ip;
mod mac;
mod output;
mod port;
mod random;
mod sanitizers;
mod ssh;
mod table;
mod text;
mod user;

pub use config::{
    build_client_websocket_url, build_websocket_url, create_client_config, create_config,
    default_config, load_client_config, load_config,
};
pub use dhcp::get_dhcp_leases;
pub use dns::{default_dns, default_dns_dual_stack};
pub use emoji::{Emoji, emoji_error, emoji_success, emoji_warning};
pub use encode::{base64_decode, base64_encode, base64_encode_file};
pub use env::{get_server_url, read_env_file_value};
pub use file_system::{
    check_file_size, copy_file, create_dir, create_file, delete_dirs, dir_exists, expand_path,
    file_exists, get_cwd, load_file, path_to_string,
};
#[cfg(unix)]
pub use file_system::{
    copy_to_dos_image, copy_to_ext4_image, create_config_archive, create_panos_bootstrap_iso,
    create_symlink, create_ztp_iso, fix_permissions_recursive, set_file_permissions,
};
pub use host::{get_fqdn, get_hostname};
#[cfg(feature = "netinfo")]
pub use host::{get_non_loopback_ipv4_addresses, get_non_loopback_ipv6_addresses};
pub use interface::{
    interface_from_idx, interface_to_idx, node_model_interfaces, srlinux_to_linux_interface,
};
pub use ip::{
    allocate_ipv6_loopback_subnet, allocate_ipv6_management_subnet, allocate_loopback_subnet,
    allocate_management_subnet, get_ip, get_ipv4_addr, get_ipv4_network, get_ipv6_addr,
    get_ipv6_ip, get_ipv6_network,
};
#[cfg(feature = "netinfo")]
pub use ip::{get_free_subnet, get_interface_networks};
pub use mac::{clean_mac, random_mac};
pub use output::{
    display_destroy_results, term_msg_highlight, term_msg_surround, term_msg_underline,
};
pub use port::id_to_port;
pub use random::{generate_lab_name, get_id, get_id_for_user};
pub use sanitizers::dasher;
pub use ssh::{
    add_lab_ssh_include, find_user_ssh_keys, generate_ssh_keypair, get_ssh_public_key,
    pub_ssh_key_to_md5_hash, pub_ssh_key_to_sha256_hash, remove_lab_ssh_include,
};
pub use table::{
    CertificateTableInfo, render_bridges_table, render_certificates_table, render_devices_table,
    render_image_detail_table, render_images_table, render_lab_info_table, render_links_table,
    render_nodes_table, render_scanned_images_table, render_server_status_table,
};
pub use text::split_node_int;
pub use user::{get_username, sherpa_user};
