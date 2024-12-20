mod connection;
mod device;

pub use connection::{
    check_duplicate_interface_link, check_interface_bounds, check_link_device, check_mgmt_usage,
};
pub use device::check_duplicate_device;
