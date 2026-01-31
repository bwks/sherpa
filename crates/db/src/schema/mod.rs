//! Database schema definitions
//!
//! This module contains the SurrealDB schema definitions for all tables
//! used by the Sherpa application. Each table schema is defined in its own
//! file for better organization and maintainability.
//!
//! ## Schema Organization
//!
//! - `helpers`: Utility functions for schema generation (enum serialization)
//! - `user`: User account table schema
//! - `node_config`: Node configuration template table schema
//! - `lab`: Network lab table schema
//! - `node`: Network node table schema
//! - `link`: Network link (connection) table schema
//! - `apply`: Schema application and orchestration
//!
//! ## Usage
//!
//! The primary entry point is the `apply_schema` function, which creates
//! all tables in the correct dependency order:
//!
//! ```no_run
//! use db::{connect, apply_schema};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let db = connect("localhost", 8000, "test", "test").await?;
//!     apply_schema(&db).await?;
//!     Ok(())
//! }
//! ```

mod apply;
mod helpers;
mod lab;
mod link;
mod node;
mod node_config;
mod user;

// Public API - only schema application function is exposed outside the crate
pub use apply::apply_schema;
