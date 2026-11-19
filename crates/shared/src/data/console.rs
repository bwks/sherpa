//! The answer to a node console check: `GET .../console?check=true` or `.../vnc?check=true`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A node console the caller may open now: the route's checks passed, and no session was taken.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleCheck {
    /// The session that holds the console, `<user> from <address> since <time> UTC`, when one
    /// does: a connection made now is refused, or takes it over with `force`.
    #[serde(default)]
    pub held_by: Option<String>,
}
