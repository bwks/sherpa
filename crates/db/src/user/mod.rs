mod create;
mod delete;
mod device_password;
mod lock;
mod read;
mod update;

// Public exports - CREATE operations
pub use create::{UsernameTaken, create_admin_user, create_user};

// Public exports - READ operations
pub use read::{find_user, find_user_by_id, get_user, list_users};

// Public exports - UPDATE operations
pub use update::{UserChanged, change_user_password, update_user};

// Public exports - the device password
pub use device_password::{
    DevicePassword, device_password_set, open_device_password, set_device_password,
};

// Public exports - the account lockout
pub use lock::{list_locked_usernames, unlock_user};

// Public exports - DELETE operations
pub use delete::delete_user;
