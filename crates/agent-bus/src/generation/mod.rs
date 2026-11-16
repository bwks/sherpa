//! The `## Generation` section of a pull request body, kept by the host: what the agent tasks
//! and the host itself spent on the work item the pull request belongs to.
//!
//! - `placement`: which pull request counts each task and each response of the host, without
//!   I/O.
//! - `section`: the section's text and data, between its markers, without I/O.
//! - `sync`: which sections are due, and writing them with `gh`.

pub mod placement;
pub mod section;
pub mod sync;
