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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_console_has_no_holder() {
        let check: ConsoleCheck = serde_json::from_str(r#"{"held_by":null}"#).unwrap();
        assert_eq!(check, ConsoleCheck::default());
        let check: ConsoleCheck = serde_json::from_str("{}").unwrap();
        assert_eq!(check, ConsoleCheck::default());
    }

    #[test]
    fn a_held_console_names_its_holder() {
        let check = ConsoleCheck {
            held_by: Some("alice from 192.0.2.10 since 2026-11-18 09:08:07 UTC".to_owned()),
        };
        let json = serde_json::to_string(&check).unwrap();
        assert_eq!(
            json,
            r#"{"held_by":"alice from 192.0.2.10 since 2026-11-18 09:08:07 UTC"}"#
        );
        assert_eq!(serde_json::from_str::<ConsoleCheck>(&json).unwrap(), check);
    }
}
