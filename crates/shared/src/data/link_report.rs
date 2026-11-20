use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The state a P2p link, or another data connection of a node, is left in after the node's end
/// was made again (by a cold boot, a container start or a redeploy), or as the scanner last
/// found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LinkState {
    /// Both ends exist and redirect to each other, with the link's impairments.
    Attached,
    /// The peer is down: stopped, paused or gone, so its end does not exist or is left alone.
    /// Nothing is wrong: the surviving end has no carrier, and the link is attached both ways,
    /// with carrier, when the peer starts again.
    PeerDown { peer: String },
    /// The link could not be attached, and why. The node is up; the link carries no traffic,
    /// and neither end has carrier, until it is attached again: by resuming or redeploying
    /// either end, or by the scanner when it finds an end it can repair.
    Failed { reason: String },
}

/// What happened to one of a node's links when its end was made again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LinkReport {
    /// The link, as `node::interface <-> node::interface`.
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
