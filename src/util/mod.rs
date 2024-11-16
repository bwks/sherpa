mod encode;
mod file_system;
mod ip;
mod mac;
mod output;
mod port;
mod random;
mod ssh;
mod user;
mod validate;

pub use crate::util::encode::base64_encode;
pub use crate::util::file_system::{
    copy_file, copy_to_usb_image, create_dir, create_file, create_ztp_iso, dir_exists, expand_path,
    file_exists, fix_permissions_recursive, get_cwd,
};
pub use crate::util::ip::get_ip;
pub use crate::util::mac::random_mac;
pub use crate::util::output::{term_msg_highlight, term_msg_surround, term_msg_underline};
pub use crate::util::port::id_to_port;
pub use crate::util::random::get_id;
pub use crate::util::ssh::{
    generate_ssh_keypair, get_ssh_public_key, pub_ssh_key_to_md5_hash, pub_ssh_key_to_sha256_hash,
    DeviceIp, SshConfigTemplate,
};
pub use crate::util::user::get_username;
pub use crate::util::validate::tcp_connect;
