//! The host's own usage and time, from Claude Code's transcripts of the host's project.
//!
//! - `transcript`: what one transcript line says, without I/O.
//! - `import`: `host import`, reading the transcripts into the record with a cursor per file.
//! - `hook`: `host import --hook`, the `SessionEnd` hook's contract.

pub mod hook;
pub mod import;
pub mod transcript;
