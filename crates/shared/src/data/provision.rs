use std::fmt;

/// What happened when the CLI provisioned a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvisionStatus {
    Provisioned,
    Failed,
    Skipped,
}

impl fmt::Display for ProvisionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Provisioned => "provisioned",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        };
        f.write_str(text)
    }
}

/// The provisioning result for one node.
#[derive(Clone, Debug)]
pub struct ProvisionResult {
    pub node: String,
    pub status: ProvisionStatus,
    pub detail: String,
}
