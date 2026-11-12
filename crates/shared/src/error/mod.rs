//! Error types for the Sherpa shared library

mod memory;
mod rpc;

pub use memory::MemoryShortfall;
pub use rpc::RpcErrorCode;
