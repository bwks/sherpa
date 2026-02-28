mod create;
mod delete;
mod read;
mod update;

// Public exports - CREATE operations
pub use create::{create_user, upsert_user};

// Public exports - READ operations
pub use read::{count_users, get_user, get_user_by_id, get_user_for_auth, list_users};

// Public exports - UPDATE operations
pub use update::update_user;

// Public exports - DELETE operations
pub use delete::{delete_user, delete_user_by_username, delete_user_safe};
