use serde::{Deserialize, Serialize};

/// What a host CPU is pinned to: a node's vCPUs, or its QEMU emulator and disk I/O threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuPinRole {
    Vcpu,
    Thread,
}

/// Host CPUs an operation asked for that another node holds: the `data` of a
/// [`CpuPinsHeld`](super::RpcErrorCode::CpuPinsHeld) error. It names no lab or node; the
/// message does, to those allowed to see them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuPinConflict {
    /// The host CPUs in conflict, in ascending order.
    pub host_cpus: Vec<u16>,
    /// What the refused node would pin to them.
    pub requested_role: CpuPinRole,
    /// What the holding node pins to them.
    pub held_role: CpuPinRole,
}

impl CpuPinRole {
    /// The role as the user reads it in a message.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Vcpu => "vCPUs",
            Self::Thread => "QEMU threads",
        }
    }
}
