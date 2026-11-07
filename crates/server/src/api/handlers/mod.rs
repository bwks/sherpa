//! HTTP handlers: the REST API and the web UI, by resource.

mod admin_web;
mod auth;
mod images_api;
mod images_web;
mod labs_api;
mod labs_web;
mod profile;
mod system;
mod users_api;

pub use admin_web::*;
pub use auth::*;
pub use images_api::*;
pub use images_web::*;
pub use labs_api::*;
pub use labs_web::*;
pub use profile::*;
pub use system::*;
pub use users_api::*;
