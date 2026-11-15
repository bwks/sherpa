//! The host CPUs nodes hold, in the `cpu_pin` table.
//!
//! Admission claims a node's pins; a redeploy claims its new ones beside its old ones, then
//! keeps one set. Deleting the node or its lab gives them back. The table's trigger refuses a
//! CPU another node holds against the pinning rules, which [`claim_cpu_pins`] reports as
//! [`CpuPinTaken`].

mod read;
mod write;

pub use read::list_cpu_pins;
pub use write::{CpuPinTaken, claim_cpu_pins, retain_cpu_pins};
