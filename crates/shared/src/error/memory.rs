use serde::{Deserialize, Serialize};

/// How far the memory an operation asked for is over what the server can give guests: the
/// `data` of an [`InsufficientMemory`](super::RpcErrorCode::InsufficientMemory) error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryShortfall {
    pub requested_mib: u64,
    pub available_mib: u64,
    pub numa_node: Option<u32>,
}
