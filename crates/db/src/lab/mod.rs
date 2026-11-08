mod create;
mod delete;
mod read;
mod update;

pub use create::{LabDetails, LabNetworks, create_lab, validate_lab_id};
pub use delete::delete_lab;
pub use read::{
    count_labs_by_user, find_lab, get_lab, get_used_ipv6_loopback_networks,
    get_used_ipv6_management_networks, get_used_loopback_networks, get_used_management_networks,
    list_labs, list_labs_by_user,
};
pub use update::{update_lab_state, update_lab_tailscale_container};
