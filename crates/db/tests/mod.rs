//! Integration tests for the db crate.
//!
//! Each test opens its own in-memory database (`helper::setup_db`), so the tests need no
//! running database server, are isolated from each other and run in parallel.
//!
//! To run them:
//! - All db tests: `cargo test --package db`
//! - One group: `cargo test --package db --test mod lab::`
//! - One test: `cargo test --package db --test mod test_get_node_image_by_id`
mod helper;

/// Node image CRUD operations
mod node_image;

/// Lab CRUD operations
mod lab;

/// User CRUD operations
mod user;

/// Node CRUD operations
mod node;

/// Link CRUD operations
mod link;

/// Schema and seeding
mod schema;

/// Cross-table relationships: cascade and restrict deletes, transactions, concurrency
mod relationships;

use helper::{create_test_config, create_test_node_with_model, ipv4_lab_networks, setup_db};

/// The one-time SurrealDB importer, against the test SurrealDB
///
/// To run: cargo test --package db import -- --ignored
mod import;
