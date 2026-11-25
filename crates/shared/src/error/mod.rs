//! Error types for the Sherpa shared library

mod cpu_pins;
mod memory;
mod rpc;
mod shared;

pub use cpu_pins::{CpuPinConflict, CpuPinRole};
pub use memory::MemoryShortfall;
pub use rpc::RpcErrorCode;
pub use shared::SharedError;
