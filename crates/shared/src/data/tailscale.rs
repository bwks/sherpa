use std::fmt;

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::instrument;

pub const TAILNET_DOCS_URL: &str = "https://github.com/bwks/sherpa/blob/main/docs/TAILNET.md";

/// Enrollment credential. Debug output must never expose its contents.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct TailnetAuthKey(String);

impl fmt::Debug for TailnetAuthKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TailnetAuthKey([REDACTED])")
    }
}

impl TailnetAuthKey {
    #[instrument(skip(value), level = "debug")]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[instrument(skip(self), level = "debug")]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TailnetState {
    Connected,
    Stopped,
    NeedsAuthentication,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteApproval {
    Approved,
    Pending,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TailnetStatus {
    pub state: TailnetState,
    pub routes: Vec<String>,
    pub approval: RouteApproval,
    pub tailscale_ips: Vec<String>,
    pub warnings: Vec<String>,
}

// Parse only the status fields we need. Daemon diagnostics can contain sensitive data.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DaemonStatus {
    backend_state: String,
    #[serde(rename = "TailscaleIPs", default)]
    tailscale_ips: Option<Vec<String>>,
    #[serde(rename = "Self")]
    own: Option<OwnStatus>,
}

#[derive(Deserialize)]
struct OwnStatus {
    #[serde(rename = "AllowedIPs")]
    allowed_ips: Option<Vec<String>>,
    #[serde(rename = "Expired", default)]
    expired: bool,
}

impl TailnetStatus {
    #[instrument(level = "debug")]
    pub fn unavailable(routes: Vec<String>) -> Self {
        Self {
            state: TailnetState::Unavailable,
            routes,
            approval: RouteApproval::Unknown,
            tailscale_ips: Vec::new(),
            warnings: vec![format!(
                "Unable to verify Tailscale gateway or route approval. See {TAILNET_DOCS_URL}#route-approval"
            )],
        }
    }

    #[instrument(skip(input), level = "debug")]
    pub fn from_daemon(input: &str, routes: Vec<String>) -> Result<Self> {
        let daemon: DaemonStatus =
            serde_json::from_str(input).context("Invalid Tailscale status response")?;
        let mut result = Self::unavailable(routes);
        result.tailscale_ips = daemon.tailscale_ips.unwrap_or_default();
        result.state = match daemon.backend_state.as_str() {
            "Running" if !daemon.own.as_ref().is_some_and(|s| s.expired) => TailnetState::Connected,
            "NeedsLogin" | "NeedsMachineAuth" | "Running" => TailnetState::NeedsAuthentication,
            "Stopped" => TailnetState::Stopped,
            _ => TailnetState::Unavailable,
        };
        if result.state == TailnetState::Connected {
            if let Some(allowed) = daemon.own.and_then(|s| s.allowed_ips) {
                result.approval = if result.routes.iter().all(|r| allowed.contains(r)) {
                    RouteApproval::Approved
                } else {
                    RouteApproval::Pending
                };
            }
            result.warnings = match result.approval {
                RouteApproval::Approved => Vec::new(),
                RouteApproval::Pending => vec![format!(
                    "Tailscale connected, but approval is pending for one or more management routes ({}). Approve the routes manually or configure autoApprovers. See {TAILNET_DOCS_URL}#route-approval",
                    result.routes.join(", ")
                )],
                RouteApproval::Unknown => result.warnings,
            };
        } else if result.state == TailnetState::NeedsAuthentication {
            result.warnings = vec![format!(
                "Tailscale gateway requires authentication or device approval. See {TAILNET_DOCS_URL}#recovery"
            )];
        } else if result.state == TailnetState::Stopped {
            result.warnings.clear();
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_redacted_in_debug_but_transmitted() {
        let key = TailnetAuthKey::new("test-secret".into());
        assert!(!format!("{key:?}").contains("test-secret"));
        assert_eq!(serde_json::to_string(&key).unwrap(), "\"test-secret\"");
    }

    #[test]
    fn approval_requires_every_advertised_route() {
        let routes = vec!["192.0.2.0/24".into(), "2001:db8::/64".into()];
        let approved = TailnetStatus::from_daemon(
            r#"{"BackendState":"Running","Self":{"AllowedIPs":["192.0.2.0/24","2001:db8::/64"]}}"#,
            routes.clone(),
        )
        .unwrap();
        assert_eq!(approved.approval, RouteApproval::Approved);
        assert!(approved.warnings.is_empty());
        let pending = TailnetStatus::from_daemon(
            r#"{"BackendState":"Running","Self":{"AllowedIPs":["192.0.2.0/24"]}}"#,
            routes.clone(),
        )
        .unwrap();
        assert_eq!(pending.approval, RouteApproval::Pending);
        assert_eq!(pending.state, TailnetState::Connected);
        assert!(pending.warnings[0].contains(TAILNET_DOCS_URL));
        let unknown = TailnetStatus::from_daemon(r#"{"BackendState":"Running"}"#, routes).unwrap();
        assert_eq!(unknown.approval, RouteApproval::Unknown);
        assert!(unknown.warnings[0].contains(TAILNET_DOCS_URL));
    }

    #[test]
    fn expired_identity_is_not_connected() {
        let status = TailnetStatus::from_daemon(
            r#"{"BackendState":"Running","Self":{"Expired":true}}"#,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(status.state, TailnetState::NeedsAuthentication);
        assert_ne!(status.approval, RouteApproval::Approved);
    }
    #[test]
    fn unauthenticated_daemon_can_report_null_addresses() {
        let status = TailnetStatus::from_daemon(
            r#"{"BackendState":"NeedsLogin","TailscaleIPs":null,"Self":null}"#,
            vec!["192.0.2.0/24".into()],
        )
        .unwrap();
        assert_eq!(status.state, TailnetState::NeedsAuthentication);
        assert!(status.warnings[0].contains("#recovery"));
    }
}
