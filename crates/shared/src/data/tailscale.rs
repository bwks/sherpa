use std::fmt;

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::konst::TAILNET_DOCS_URL;

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

    /// The tailnet status from the Tailscale daemon's JSON status, for a gateway advertising
    /// `routes`.
    ///
    /// # Errors
    ///
    /// Fails when `input` is not the daemon's JSON status.
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

    fn both_routes() -> Vec<String> {
        vec!["192.0.2.0/24".into(), "2001:db8::/64".into()]
    }

    #[test]
    fn tailnet_key_006_debug_output_is_redacted_and_the_request_carries_the_key() {
        let key = TailnetAuthKey::new("tskey-auth-testsecret".into());
        let debug = format!("{key:?}");
        assert!(debug.contains("[REDACTED]"), "{debug}");
        assert!(!debug.contains("tskey-auth-testsecret"));
        assert_eq!(
            serde_json::to_string(&key).unwrap(),
            r#""tskey-auth-testsecret""#
        );
    }

    #[test]
    fn tailnet_status_001_every_advertised_route_allowed_reads_as_approved_with_no_warning() {
        let status = TailnetStatus::from_daemon(
            r#"{"BackendState":"Running","Self":{"AllowedIPs":["192.0.2.0/24","2001:db8::/64"]}}"#,
            both_routes(),
        )
        .unwrap();
        assert_eq!(status.state, TailnetState::Connected);
        assert_eq!(status.approval, RouteApproval::Approved);
        assert_eq!(status.warnings, Vec::<String>::new());
    }

    #[test]
    fn tailnet_status_002_one_route_not_allowed_reads_as_pending_with_a_warning_naming_the_routes()
    {
        let status = TailnetStatus::from_daemon(
            r#"{"BackendState":"Running","Self":{"AllowedIPs":["192.0.2.0/24"]}}"#,
            both_routes(),
        )
        .unwrap();
        assert_eq!(status.state, TailnetState::Connected);
        assert_eq!(status.approval, RouteApproval::Pending);
        let warning = &status.warnings[0];
        assert!(warning.contains("192.0.2.0/24"), "{warning}");
        assert!(warning.contains("2001:db8::/64"), "{warning}");
        assert!(warning.contains("#route-approval"), "{warning}");
    }

    #[test]
    fn tailnet_status_003_no_allowed_addresses_reported_reads_as_unknown_with_a_warning() {
        let status =
            TailnetStatus::from_daemon(r#"{"BackendState":"Running"}"#, both_routes()).unwrap();
        assert_eq!(status.approval, RouteApproval::Unknown);
        assert!(status.warnings[0].contains("#route-approval"));
    }

    #[test]
    fn tailnet_status_004_an_expired_key_reads_as_needing_authentication_and_never_approved() {
        let status = TailnetStatus::from_daemon(
            r#"{"BackendState":"Running","Self":{"Expired":true,"AllowedIPs":["192.0.2.0/24"]}}"#,
            vec!["192.0.2.0/24".into()],
        )
        .unwrap();
        assert_eq!(status.state, TailnetState::NeedsAuthentication);
        assert_ne!(status.approval, RouteApproval::Approved);
    }

    #[test]
    fn tailnet_status_005_a_logged_out_daemon_reads_as_needing_authentication_with_a_recovery_pointer()
     {
        let status = TailnetStatus::from_daemon(
            r#"{"BackendState":"NeedsLogin","TailscaleIPs":null,"Self":null}"#,
            vec!["192.0.2.0/24".into()],
        )
        .unwrap();
        assert_eq!(status.state, TailnetState::NeedsAuthentication);
        assert!(status.warnings[0].contains("#recovery"));
    }
}
