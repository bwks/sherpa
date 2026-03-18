use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use shared::data::{ClientConfig, StatusKind, StatusMessage, UpResponse};
use shared::error::RpcErrorCode;
use shared::konst::{LAB_FILE_NAME, SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_FILE};
use shared::util::{
    Emoji, base64_encode, get_cwd, get_username, render_lab_info_table, render_nodes_table,
    term_msg_surround, term_msg_underline,
};
use topology::StartupScript;

use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Helper struct for deserializing log messages
#[derive(Deserialize)]
struct LogMessage {
    r#type: String,
    message: String,
}

/// Start lab  to sherpad server with streaming progress updates
///
/// Flow:
/// 1. Load manifest from TOML file
/// 2. Convert manifest to JSON
/// 3. Connect to server with extended timeout (15 minutes for long-running operation)
/// 4. Send RPC request and stream progress updates
/// 5. Display final results
pub async fn up(
    lab_name: &str,
    lab_id: &str,
    manifest_path: &str,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    term_msg_surround(&format!("Start Lab - {lab_name}-{lab_id}"));

    // Load authentication token
    let token = match load_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("\n{} Authentication required", Emoji::Error);
            eprintln!("   Please run: sherpa login");
            eprintln!("   Error: {}", e);
            bail!("Authentication token not found");
        }
    };

    // Load and parse manifest
    println!("\nLoading manifest from: {}\n", manifest_path);

    let mut manifest = topology::Manifest::load_file(manifest_path)
        .with_context(|| format!("Failed to parse manifest: {}", manifest_path))?;

    // Read per-node ztp_config file paths and base64 encode their contents
    resolve_ztp_configs(&mut manifest, manifest_path)?;

    // Resolve startup_scripts paths relative to manifest directory
    resolve_startup_scripts(&mut manifest, manifest_path)?;

    // Resolve $ENV_VAR references in environment_variables
    resolve_environment_variables(&mut manifest)?;

    // Serialize manifest to JSON for transmission
    let manifest_value =
        serde_json::to_value(&manifest).context("Failed to serialize manifest to JSON")?;

    // Extended timeout for long-running up operation (15 minutes)
    let timeout = Duration::from_secs(900);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    // Connect
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    // Get current username (for display only - server uses token)
    let _username = get_username()?;

    // Create RPC request with authentication token
    let up_request = RpcRequest::new(
        "up",
        serde_json::json!({
            "lab_id": lab_id,
            "manifest": manifest_value,
            "token": token,
        }),
    );

    println!("Starting lab creation...\n");

    // Call streaming RPC with progress callback
    let up_response = rpc_client
        .call_streaming(up_request, |msg_text| {
            // Parse and display progress messages
            if let Ok(status_msg) = serde_json::from_str::<StatusMessage>(msg_text) {
                if status_msg.r#type == "status" {
                    // Display phase header if available
                    if let Some(phase) = &status_msg.phase {
                        println!(); // Blank line before phase header
                        term_msg_underline(phase);
                    }
                    // Display the message with kind-appropriate emoji
                    let emoji = match status_msg.kind {
                        StatusKind::Progress => Emoji::Progress,
                        StatusKind::Done => Emoji::Success,
                        StatusKind::Info => Emoji::Info,
                        StatusKind::Waiting => Emoji::Hourglass,
                    };
                    println!("{} {}", emoji, status_msg.message);
                }
            } else if let Ok(log_msg) = serde_json::from_str::<LogMessage>(msg_text)
                && log_msg.r#type == "log"
            {
                // Display log messages (for debugging)
                println!("[LOG] {}", log_msg.message);
            }
        })
        .await
        .context("Up RPC call failed")?;

    // Close connection
    rpc_client.close().await.ok();

    // Handle errors
    if let Some(error) = up_response.error {
        eprintln!("\n{} Server Error:", Emoji::Error);
        eprintln!("   Message: {}", error.message);
        eprintln!("   Code: {}", error.code);
        if let Some(context) = error.context {
            eprintln!("   Context:\n{}", context);
        }

        // Check for authentication errors
        if error.code == RpcErrorCode::AuthRequired {
            eprintln!(
                "\n{} Your authentication token has expired or is invalid",
                Emoji::Error
            );
            eprintln!("   Please run: sherpa login");
        }

        bail!("Lab creation failed");
    }

    let up_result = up_response.result.context("No result in up response")?;
    let up_data: UpResponse =
        serde_json::from_value(up_result).context("Failed to parse up response")?;

    // Write SSH config to local directory
    match get_cwd() {
        Ok(cwd) => {
            let local_ssh_config_path = format!("{}/{}", cwd, SHERPA_SSH_CONFIG_FILE);
            match fs::write(&local_ssh_config_path, &up_data.ssh_config) {
                Ok(_) => {
                    println!(
                        "\n{} SSH config created: {}",
                        Emoji::Success,
                        local_ssh_config_path
                    );
                }
                Err(e) => {
                    println!(
                        "\n{} Warning: Failed to create local SSH config: {}",
                        Emoji::Warning,
                        e
                    );
                }
            }
        }
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {}",
                Emoji::Warning,
                e
            );
        }
    }

    // Write SSH private key to local directory with 0600 permissions
    match get_cwd() {
        Ok(cwd) => {
            let local_ssh_key_path = format!("{}/{}", cwd, SHERPA_SSH_PRIVATE_KEY_FILE);
            match fs::write(&local_ssh_key_path, &up_data.ssh_private_key) {
                Ok(_) => {
                    // Set Unix permissions to 0600 (owner read/write only)
                    #[cfg(unix)]
                    {
                        if let Err(e) = fs::set_permissions(
                            &local_ssh_key_path,
                            fs::Permissions::from_mode(0o600),
                        ) {
                            println!(
                                "\n{} Warning: Failed to set permissions on SSH key: {}",
                                Emoji::Warning,
                                e
                            );
                        }
                    }
                    println!(
                        "{} SSH private key created: {}",
                        Emoji::Success,
                        local_ssh_key_path
                    );
                }
                Err(e) => {
                    println!(
                        "\n{} Warning: Failed to create local SSH private key: {}",
                        Emoji::Warning,
                        e
                    );
                }
            }
        }
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {}",
                Emoji::Warning,
                e
            );
        }
    }

    // Write lab-info.toml to local directory
    match get_cwd() {
        Ok(cwd) => {
            let local_lab_info_path = format!("{}/{}", cwd, LAB_FILE_NAME);
            match fs::write(&local_lab_info_path, up_data.lab_info.to_string()) {
                Ok(_) => {
                    println!(
                        "{} Lab info created: {}",
                        Emoji::Success,
                        local_lab_info_path
                    );
                }
                Err(e) => {
                    println!(
                        "\n{} Warning: Failed to create local lab info: {}",
                        Emoji::Warning,
                        e
                    );
                }
            }
        }
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {}",
                Emoji::Warning,
                e
            );
        }
    }

    // Display results
    display_up_results(&up_data)?;

    Ok(())
}

/// Resolve `ztp_config` file paths in manifest nodes.
///
/// For each node with a `ztp_config` field, the value is treated as a file path.
/// The file is read, base64 encoded, and the value is replaced with the encoded contents.
/// Relative paths are resolved from the manifest file's parent directory.
pub(crate) fn resolve_ztp_configs(
    manifest: &mut topology::Manifest,
    manifest_path: &str,
) -> Result<()> {
    let manifest_dir = Path::new(manifest_path).parent().unwrap_or(Path::new("."));

    for node in &mut manifest.nodes {
        let ztp_path_str = match &node.ztp_config {
            Some(s) => s.clone(),
            None => continue,
        };

        let ztp_path = Path::new(&ztp_path_str);
        let resolved_path = if ztp_path.is_absolute() {
            ztp_path.to_path_buf()
        } else {
            manifest_dir.join(ztp_path)
        };

        let contents = fs::read_to_string(&resolved_path).with_context(|| {
            format!(
                "Failed to read ztp_config for node '{}': {}",
                node.name,
                resolved_path.display()
            )
        })?;

        if contents.is_empty() {
            bail!(
                "ztp_config for node '{}' is empty: {}",
                node.name,
                resolved_path.display()
            );
        }

        node.ztp_config = Some(base64_encode(&contents));
    }

    Ok(())
}

/// Resolve startup_scripts file paths relative to the manifest directory.
/// Reads each script file, base64-encodes the contents, and populates
/// `startup_scripts_data` for transmission to the server.
pub(crate) fn resolve_startup_scripts(
    manifest: &mut topology::Manifest,
    manifest_path: &str,
) -> Result<()> {
    let manifest_dir = Path::new(manifest_path).parent().unwrap_or(Path::new("."));

    for node in &mut manifest.nodes {
        let scripts = match &node.startup_scripts {
            Some(s) if !s.is_empty() => s.clone(),
            _ => continue,
        };

        let mut script_data = Vec::with_capacity(scripts.len());
        for script_path_str in &scripts {
            let script_path = Path::new(script_path_str);
            let resolved_path = if script_path.is_absolute() {
                script_path.to_path_buf()
            } else {
                manifest_dir.join(script_path)
            };

            let contents = fs::read_to_string(&resolved_path).with_context(|| {
                format!(
                    "Failed to read startup_script for node '{}': {}",
                    node.name,
                    resolved_path.display()
                )
            })?;

            if contents.is_empty() {
                bail!(
                    "startup_script for node '{}' is empty: {}",
                    node.name,
                    resolved_path.display()
                );
            }

            let filename = resolved_path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| script_path_str.clone());

            script_data.push(StartupScript {
                filename,
                content: base64_encode(&contents),
            });
        }

        node.startup_scripts_data = Some(script_data);
    }

    Ok(())
}

/// Resolve `$ENV_VAR` references in environment variable values.
///
/// For each node's `environment_variables`, entries of the form `KEY=$VAR`
/// have the `$VAR` portion resolved from the user's shell environment.
/// Only exact `$VAR` values are expanded (not partial references like `prefix$VAR`).
pub(crate) fn resolve_environment_variables(manifest: &mut topology::Manifest) -> Result<()> {
    for node in &mut manifest.nodes {
        let env_vars = match &mut node.environment_variables {
            Some(vars) if !vars.is_empty() => vars,
            _ => continue,
        };

        for entry in env_vars.iter_mut() {
            let Some((key, value)) = entry.split_once('=') else {
                bail!(
                    "Invalid environment variable '{}' for node '{}': must be KEY=VALUE format",
                    entry,
                    node.name
                );
            };

            if let Some(var_name) = value.strip_prefix('$') {
                let resolved = std::env::var(var_name).with_context(|| {
                    format!(
                        "Environment variable '{}' referenced in node '{}' is not set",
                        var_name, node.name
                    )
                })?;
                *entry = format!("{}={}", key, resolved);
            }
        }
    }

    Ok(())
}

/// Display lab creation results
fn display_up_results(response: &UpResponse) -> Result<()> {
    println!();
    term_msg_surround("Lab Creation Results");

    // Lab Info
    let lab_info_table = render_lab_info_table(&response.lab_info);
    println!("{}", lab_info_table);

    // Node information
    if !response.nodes.is_empty() {
        println!("\n{}", render_nodes_table(&response.nodes));
    }

    // Timing
    println!("\nDuration: {:.2}s", response.total_time_secs);

    // Errors (if any)
    if !response.errors.is_empty() {
        println!("\n{} Warnings/Errors:", Emoji::Warning);
        for error in &response.errors {
            let icon = if error.is_critical {
                Emoji::Error
            } else {
                Emoji::Warning
            };
            println!("  {} [{}] {}", icon, error.phase, error.message);
        }
    }

    // Final status
    if response.success {
        println!("\n{} Lab created successfully!\n", Emoji::Success);
    } else {
        println!(
            "\n{} Lab partially created - review errors above\n",
            Emoji::Warning
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeModel;
    use shared::util::base64_decode;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn test_manifest(nodes: Vec<topology::Node>) -> topology::Manifest {
        topology::Manifest {
            name: "test-lab".to_string(),
            nodes,
            ..Default::default()
        }
    }

    #[test]
    fn test_resolve_ztp_configs_base64_encodes_content() {
        let mut config_file = NamedTempFile::new().expect("Failed to create temp file");
        write!(config_file, "hostname dev01").expect("Failed to write");
        let config_path = config_file.path().to_str().expect("path").to_string();

        let mut manifest = test_manifest(vec![
            topology::Node {
                name: "dev01".to_string(),
                model: NodeModel::CiscoCat8000v,
                ztp_config: Some(config_path),
                ..Default::default()
            },
            topology::Node {
                name: "dev02".to_string(),
                model: NodeModel::CiscoIosv,
                ..Default::default()
            },
        ]);

        resolve_ztp_configs(&mut manifest, "/tmp/manifest.toml").expect("resolve should succeed");

        // dev01: ztp_config is now base64 encoded contents
        let encoded = manifest.nodes[0]
            .ztp_config
            .as_ref()
            .expect("should have value");
        let decoded = base64_decode(encoded).expect("should decode");
        assert_eq!(decoded, "hostname dev01");

        // dev02: unchanged, no ztp_config
        assert!(manifest.nodes[1].ztp_config.is_none());
    }

    #[test]
    fn test_resolve_ztp_configs_relative_path() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let config_path = dir.path().join("my_config.txt");
        std::fs::write(&config_path, "interface eth0").expect("write config");

        let manifest_path = dir.path().join("manifest.toml");

        let mut manifest = test_manifest(vec![topology::Node {
            name: "r01".to_string(),
            model: NodeModel::AristaVeos,
            ztp_config: Some("my_config.txt".to_string()),
            ..Default::default()
        }]);

        resolve_ztp_configs(&mut manifest, manifest_path.to_str().expect("path"))
            .expect("resolve should succeed");

        let encoded = manifest.nodes[0]
            .ztp_config
            .as_ref()
            .expect("should have value");
        let decoded = base64_decode(encoded).expect("should decode");
        assert_eq!(decoded, "interface eth0");
    }

    #[test]
    fn test_resolve_ztp_configs_empty_file_error() {
        let config_file = NamedTempFile::new().expect("Failed to create temp file");
        let config_path = config_file.path().to_str().expect("path").to_string();

        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::CiscoCat8000v,
            ztp_config: Some(config_path),
            ..Default::default()
        }]);

        let result = resolve_ztp_configs(&mut manifest, "/tmp/manifest.toml");
        assert!(result.is_err());
        let err_msg = format!("{}", result.err().expect("should be error"));
        assert!(err_msg.contains("empty"));
    }

    #[test]
    fn test_resolve_ztp_configs_missing_file_error() {
        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::CiscoCat8000v,
            ztp_config: Some("/nonexistent/path/config.txt".to_string()),
            ..Default::default()
        }]);

        let result = resolve_ztp_configs(&mut manifest, "/tmp/manifest.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_environment_variables_literal_values() {
        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            environment_variables: Some(vec![
                "EDITOR=vim".to_string(),
                "LANG=en_US.UTF-8".to_string(),
            ]),
            ..Default::default()
        }]);

        resolve_environment_variables(&mut manifest).expect("should succeed");

        let env_vars = manifest.nodes[0]
            .environment_variables
            .as_ref()
            .expect("should have env vars");
        assert_eq!(env_vars[0], "EDITOR=vim");
        assert_eq!(env_vars[1], "LANG=en_US.UTF-8");
    }

    #[test]
    fn test_resolve_environment_variables_env_ref() {
        // SAFETY: test-only, run with --test-threads=1
        unsafe { std::env::set_var("SHERPA_TEST_TOKEN", "secret123") };

        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            environment_variables: Some(vec!["MY_TOKEN=$SHERPA_TEST_TOKEN".to_string()]),
            ..Default::default()
        }]);

        resolve_environment_variables(&mut manifest).expect("should succeed");

        let env_vars = manifest.nodes[0]
            .environment_variables
            .as_ref()
            .expect("should have env vars");
        assert_eq!(env_vars[0], "MY_TOKEN=secret123");

        // SAFETY: test-only cleanup
        unsafe { std::env::remove_var("SHERPA_TEST_TOKEN") };
    }

    #[test]
    fn test_resolve_environment_variables_missing_env_var() {
        // SAFETY: test-only, ensuring var is not set
        unsafe { std::env::remove_var("SHERPA_NONEXISTENT_VAR_12345") };

        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            environment_variables: Some(vec!["MY_VAR=$SHERPA_NONEXISTENT_VAR_12345".to_string()]),
            ..Default::default()
        }]);

        let result = resolve_environment_variables(&mut manifest);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("SHERPA_NONEXISTENT_VAR_12345"));
        assert!(err_msg.contains("dev01"));
    }

    #[test]
    fn test_resolve_environment_variables_no_env_vars() {
        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            ..Default::default()
        }]);

        resolve_environment_variables(&mut manifest).expect("should succeed with no env vars");
    }

    #[test]
    fn test_resolve_environment_variables_invalid_format() {
        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            environment_variables: Some(vec!["NO_EQUALS_SIGN".to_string()]),
            ..Default::default()
        }]);

        let result = resolve_environment_variables(&mut manifest);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("KEY=VALUE"));
    }
}
