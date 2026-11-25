use std::path::Path;

use anyhow::{Result, ensure};
use shared::data::TailscaleGatewaySettings;
use tracing::instrument;

use crate::services::error::ServiceError;

/// Validate server gateway settings before any infrastructure is contacted.
#[instrument(skip(settings), level = "debug")]
pub fn validate_tailscale_gateway_settings(settings: &TailscaleGatewaySettings) -> Result<()> {
    ensure!(
        !settings.image.is_empty()
            && !settings.image.chars().any(char::is_whitespace)
            && !settings.image.contains('\0'),
        ServiceError::TailscaleImageInvalid
    );
    let socket = Path::new(&settings.socket_path);
    ensure!(
        socket.is_absolute()
            && socket.file_name().is_some()
            && !settings.socket_path.contains('\0')
            && settings.socket_path.len() < 108,
        ServiceError::TailscaleSocketPathInvalid
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
        ensure!(
            seconds > 0,
            ServiceError::TailscaleTimeoutNotPositive { field }
        );
    }
    ensure!(
        i32::try_from(settings.stop_timeout_secs).is_ok(),
        ServiceError::TailscaleStopTimeoutTooLong
    );
    ensure!(
        settings.exec_timeout_secs > settings.enrollment_timeout_secs,
        ServiceError::TailscaleExecTimeoutTooShort {
            exec_timeout_secs: settings.exec_timeout_secs,
            enrollment_timeout_secs: settings.enrollment_timeout_secs,
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailnet_settings_003_each_unusable_gateway_setting_is_refused_carrying_it() {
        type SettingsEdit = fn(&mut TailscaleGatewaySettings);
        let defaults = TailscaleGatewaySettings::default();
        assert!(validate_tailscale_gateway_settings(&defaults).is_ok());
        let timeout = |field| ServiceError::TailscaleTimeoutNotPositive { field };
        let cases: &[(ServiceError, SettingsEdit)] = &[
            (ServiceError::TailscaleImageInvalid, |s| s.image.clear()),
            (ServiceError::TailscaleImageInvalid, |s| {
                s.image = "image with spaces".into();
            }),
            (ServiceError::TailscaleSocketPathInvalid, |s| {
                s.socket_path = "relative.sock".into();
            }),
            (ServiceError::TailscaleSocketPathInvalid, |s| {
                s.socket_path = "/".into();
            }),
            (ServiceError::TailscaleSocketPathInvalid, |s| {
                s.socket_path = "/run/invalid\0.sock".into();
            }),
            (ServiceError::TailscaleSocketPathInvalid, |s| {
                s.socket_path = format!("/run/{}", "s".repeat(108));
            }),
            (timeout("exec_timeout_secs"), |s| s.exec_timeout_secs = 0),
            (timeout("daemon_ready_timeout_secs"), |s| {
                s.daemon_ready_timeout_secs = 0;
            }),
            (timeout("enrollment_timeout_secs"), |s| {
                s.enrollment_timeout_secs = 0;
            }),
            (timeout("connection_timeout_secs"), |s| {
                s.connection_timeout_secs = 0;
            }),
            (timeout("stop_timeout_secs"), |s| s.stop_timeout_secs = 0),
            (ServiceError::TailscaleStopTimeoutTooLong, |s| {
                s.stop_timeout_secs = i32::MAX as u32 + 1;
            }),
        ];
        for (expected, edit) in cases {
            let mut settings = defaults.clone();
            edit(&mut settings);
            let error = validate_tailscale_gateway_settings(&settings).unwrap_err();
            assert_eq!(error.downcast_ref::<ServiceError>(), Some(expected));
        }
    }

    #[test]
    fn tailnet_settings_009_a_command_timeout_not_longer_than_the_enrollment_timeout_is_refused_carrying_both()
     {
        for exec_timeout_secs in [60, 30] {
            let settings = TailscaleGatewaySettings {
                exec_timeout_secs,
                enrollment_timeout_secs: 60,
                ..Default::default()
            };
            let error = validate_tailscale_gateway_settings(&settings).unwrap_err();
            assert_eq!(
                error.downcast_ref::<ServiceError>(),
                Some(&ServiceError::TailscaleExecTimeoutTooShort {
                    exec_timeout_secs,
                    enrollment_timeout_secs: 60,
                })
            );
        }
        let settings = TailscaleGatewaySettings {
            exec_timeout_secs: 61,
            enrollment_timeout_secs: 60,
            ..Default::default()
        };
        assert!(validate_tailscale_gateway_settings(&settings).is_ok());
    }
}
