//! Error types for the Sherpa shared library

mod cpu_pins;
mod memory;
mod rpc;

pub use cpu_pins::{CpuPinConflict, CpuPinRole};
pub use memory::MemoryShortfall;
pub use rpc::RpcErrorCode;
