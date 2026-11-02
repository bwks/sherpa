//! Database schema definitions
//!
//! Each table's schema is in its own file. A file holds the table's `CREATE` statements and,
//! for tables with enum columns or protected rows, the triggers that enforce those rules.
//!
//! ## Schema Organization
//!
//! - `helpers`: SQL text, enum value lists and rule triggers
//! - `user`: User account table
//! - `node_image`: Node image table
//! - `lab`: Network lab table
//! - `node`: Network node table
//! - `link`: Network link table
//! - `bridge`: Private bridge and bridge member tables
//! - `apply`: Schema versions and their application
//!
//! ## Usage
//!
//! The entry point is `apply_schema`, which brings a database to the current schema version
//! and recreates the rule triggers.

mod apply;
mod bridge;
mod helpers;
mod lab;
mod link;
mod node;
mod node_image;
mod user;

// Public API - only schema application function is exposed outside the crate
pub use apply::{SCHEMA_VERSION, apply_schema};
