//! Validators shared by more than one crate.
//!
//! Rules that only one crate enforces live in that crate's own `validate` module, and
//! checks on manifest types live in `topology::validate`. Validators only inspect the data
//! they are given; they do not generate data.

mod interface_count;
mod password;
mod ssh_key;
mod tailscale_key;
mod username;

pub use interface_count::{effective_data_interface_count, validate_data_interface_count_override};
pub use password::validate_password_strength;
pub use ssh_key::validate_ssh_key;
pub use tailscale_key::validate_tailscale_key;
pub use username::{validate_guest_username, validate_username};
