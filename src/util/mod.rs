mod file_system;
mod mac;
mod output;

pub use crate::util::file_system::{create_dir, create_file, dir_exists, expand_path, file_exists};
pub use crate::util::mac::random_mac_suffix;
pub use crate::util::output::term_msg;
