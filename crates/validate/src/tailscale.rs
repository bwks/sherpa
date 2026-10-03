use std::path::Path;

use anyhow::{Result, ensure};
use shared::data::TailscaleGatewaySettings;
use topology::Manifest;
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

/// Validate configuration without resolving secrets or contacting Tailscale.
#[instrument(skip(manifest), level = "debug")]
pub fn validate_tailscale(manifest: &Manifest) -> Result<()> {
    let Some(config) = &manifest.tailscale else {
        return Ok(());
    };
    if !config.enabled {
        return Ok(());
    }
    let name = config.auth_key_env.as_deref().unwrap_or_default();
    ensure!(
        valid_env_name(name),
        "tailscale.auth_key_env must name an environment variable (letters, digits, underscores; not starting with a digit)"
    );
    Ok(())
}

#[instrument(skip(key), level = "debug")]
pub fn validate_tailscale_key(key: &str) -> Result<()> {
    ensure!(
        key.starts_with("tskey-auth-")
            && key.len() > 11
            && key.len() <= 512
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "Tailscale enrollment requires a nonempty Tailscale auth key; OAuth credentials and whitespace are not supported"
    );
    Ok(())
}

fn valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
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

    #[test]
    fn environment_reference_is_a_name_not_a_secret_or_expression() {
        for name in [
            "",
            "1KEY",
            "$KEY",
            "key-name",
            "tskey-auth-secret",
            "KEY=value",
        ] {
            assert!(!valid_env_name(name));
        }
        assert!(valid_env_name("SHERPA_TAILSCALE_KEY"));
    }

    #[test]
    fn invalid_key_errors_do_not_echo_secrets() {
        for key in [
            "",
            "tskey-auth-",
            "tskey-auth-secret\n",
            "tskey-client-secret",
            "secret",
        ] {
            let error = validate_tailscale_key(key).unwrap_err().to_string();
            if !key.is_empty() {
                assert!(!error.contains(key));
            }
        }
        assert!(validate_tailscale_key("tskey-auth-example123456789").is_ok());
    }
}
