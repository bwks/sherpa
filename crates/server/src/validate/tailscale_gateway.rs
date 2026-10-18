use std::path::Path;

use anyhow::{Result, ensure};
use shared::data::TailscaleGatewaySettings;
use tracing::instrument;

/// Validate server gateway settings before any infrastructure is contacted.
#[instrument(skip(settings), level = "debug")]
pub fn validate_tailscale_gateway_settings(settings: &TailscaleGatewaySettings) -> Result<()> {
    ensure!(
        !settings.image.is_empty()
            && !settings.image.chars().any(char::is_whitespace)
            && !settings.image.contains('\0'),
        "tailscale.image must be a nonempty container image reference without whitespace or NULs"
    );
    let socket = Path::new(&settings.socket_path);
    ensure!(
        socket.is_absolute()
            && socket.file_name().is_some()
            && !settings.socket_path.contains('\0')
            && settings.socket_path.len() < 108,
        "tailscale.socket_path must be an absolute Unix socket filename shorter than 108 bytes without NULs"
    );
    for (field, seconds) in [
        ("exec_timeout_secs", settings.exec_timeout_secs),
        (
            "daemon_ready_timeout_secs",
            settings.daemon_ready_timeout_secs,
        ),
        ("enrollment_timeout_secs", settings.enrollment_timeout_secs),
        ("connection_timeout_secs", settings.connection_timeout_secs),
        ("stop_timeout_secs", settings.stop_timeout_secs),
    ] {
        ensure!(seconds > 0, "tailscale.{field} must be positive seconds");
    }
    ensure!(
        i32::try_from(settings.stop_timeout_secs).is_ok(),
        "tailscale.stop_timeout_secs must fit a signed 32-bit Docker stop timeout"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_settings_reject_invalid_image_socket_and_timeouts() {
        let defaults = TailscaleGatewaySettings::default();
        assert!(validate_tailscale_gateway_settings(&defaults).is_ok());
        type SettingsEdit = fn(&mut TailscaleGatewaySettings);
        let cases: &[(&str, SettingsEdit)] = &[
            ("image", |s| s.image.clear()),
            ("image", |s| s.image = "image with spaces".into()),
            ("socket_path", |s| s.socket_path = "relative.sock".into()),
            ("socket_path", |s| s.socket_path = "/".into()),
            ("socket_path", |s| {
                s.socket_path = "/run/invalid\0.sock".into()
            }),
            ("socket_path", |s| {
                s.socket_path = format!("/run/{}", "s".repeat(108))
            }),
            ("exec_timeout_secs", |s| s.exec_timeout_secs = 0),
            ("daemon_ready_timeout_secs", |s| {
                s.daemon_ready_timeout_secs = 0
            }),
            ("enrollment_timeout_secs", |s| s.enrollment_timeout_secs = 0),
            ("connection_timeout_secs", |s| s.connection_timeout_secs = 0),
            ("stop_timeout_secs", |s| s.stop_timeout_secs = 0),
            ("stop_timeout_secs", |s| {
                s.stop_timeout_secs = i32::MAX as u32 + 1
            }),
        ];
        for (field, edit) in cases {
            let mut settings = defaults.clone();
            edit(&mut settings);
            let error = validate_tailscale_gateway_settings(&settings).unwrap_err();
            assert!(error.to_string().contains(field), "{error}");
        }
    }
}
