//! The crate's SQL statements, one file each under `sql/queries/`. `build.rs` prepares every one
//! against the schema in `sql/schema/` before the crate compiles, so a statement that does not fit
//! the schema fails the build, and it generates one constant per file, included here.

include!(concat!(env!("OUT_DIR"), "/queries.rs"));
