mod file_system;
mod mac;

pub use crate::util::file_system::{create_dir, create_file, dir_exists, file_exists};
pub use crate::util::mac::random_mac_suffix;
