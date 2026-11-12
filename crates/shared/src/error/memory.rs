use serde::{Deserialize, Serialize};

/// How far the memory an operation asked for is over what the server can give guests: the
/// `data` of an [`InsufficientMemory`](super::RpcErrorCode::InsufficientMemory) error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryShortfall {
    /// Memory the refused VMs and unikernels need, in MiB.
    pub requested_mib: u64,
    /// Guest memory left for them, in MiB: on the whole server, or on `numa_node`.
    pub available_mib: u64,
    /// The host NUMA node that is short, when the nodes are held strictly to one; `None` when
    /// the whole server is.
    pub numa_node: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shortfall_serializes_with_its_field_names() {
        let shortfall = MemoryShortfall {
            requested_mib: 30720,
            available_mib: 26624,
            numa_node: None,
        };
        let json = serde_json::to_value(shortfall).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "requested_mib": 30720,
                "available_mib": 26624,
                "numa_node": null,
            })
        );
        assert_eq!(
            serde_json::from_value::<MemoryShortfall>(json).unwrap(),
            shortfall
        );
    }
}
