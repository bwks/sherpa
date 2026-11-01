//! One-time import of a pre-0.5.0 server's data from SurrealDB
//!
//! Releases before 0.5.0 kept Sherpa's records in a SurrealDB server. The import reads every
//! table from that server and writes the records, with their keys, into the embedded database.
//!
//! - `rows`: the rows as SurrealDB stored them, and their conversion to Sherpa's records
//! - `source`: connecting to and reading the SurrealDB server
//! - `write`: writing the records in one transaction and checking the row counts

mod rows;
mod source;
mod write;

pub use source::SurrealSource;
pub use write::{ImportSummary, import_from_surrealdb};
