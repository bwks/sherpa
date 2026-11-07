//! JSON-RPC over the WebSocket: one dispatcher, and the methods by resource.

mod auth;
mod dispatch;
mod images;
mod labs;
mod server;
mod users;

pub use dispatch::{Access, METHODS, Method, dispatch};
