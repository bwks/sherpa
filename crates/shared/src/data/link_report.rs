use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The state a P2p link, or another data connection of a node, is left in after the node's end
/// was made again (by a cold boot, a container start or a redeploy), or as the scanner last
/// found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LinkState {
    Attached,
    PeerDown { peer: String },
    Failed { reason: String },
}

/// What happened to one of a node's links when its end was made again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LinkReport {
    pub link: String,
    #[serde(flatten)]
    pub state: LinkState,
}

impl LinkReport {
    /// Whether the link failed: it is neither attached nor waiting for its peer.
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(self.state, LinkState::Failed { .. })
    }
}

impl fmt::Display for LinkReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.state {
            LinkState::Attached => write!(f, "link {} attached", self.link),
            LinkState::PeerDown { peer } => write!(
                f,
                "link {} not attached: {peer} is down; it is attached when {peer} starts",
                self.link
            ),
            LinkState::Failed { reason } => {
                write!(f, "link {} not attached: {reason}", self.link)
            }
        }
    }
}
