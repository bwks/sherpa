//! The crate's SQL statements, one file each under `sql/queries/`. `build.rs` prepares every one
//! against the schema in `sql/schema/` before the crate compiles, so a statement that does not fit
//! the schema fails the build, and it generates one constant per file, included here. Each is a
//! `Query<N>`, `N` being the parameters the file binds, so a call binding another number of values
//! does not compile.

include!(concat!(env!("OUT_DIR"), "/queries.rs"));
