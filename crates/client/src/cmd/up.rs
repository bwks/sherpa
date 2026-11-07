use anyhow::{Context, Result, bail};
use gethostname::gethostname;
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::time::Duration;

use shared::data::{ClientConfig, NodeInfo, StatusMessage, UpResponse};
use shared::util::{
    Emoji, base64_encode, get_cwd, load_file, render_lab_info_table, render_nodes_table,
    render_tailnet_table, term_msg_surround, term_msg_underline,
};
use topology::StartupScript;

use super::progress::status_emoji;
use crate::cmd::interrupt::{
    OperationTarget, StoppableOperation, Waited, call_streaming_or_interrupt,
};
use crate::cmd::tailscale_key::resolve_tailscale_key;
use crate::lab_files::{LabFiles, register_ssh_config, save_lab_files};
use crate::provision::{
    client_variable, container_results, container_secrets, provision_lab, resolve_client_path,
};
use crate::ssh_client::HostKeyRefresh;
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};
use topology::validate::{check_text_file_modes, validate_provisioners};

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
#[tracing::instrument(skip_all, fields(%lab_name, %lab_id))]
pub async fn up(
    lab_name: &str,
    lab_id: &str,
    manifest_path: &str,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    term_msg_surround(&format!("Start Lab - {lab_name}-{lab_id}"));

    let token = load_token()?;

    // Load and parse manifest
    println!("\nLoading manifest from: {}\n", manifest_path);

    let mut manifest = topology::Manifest::load_file(manifest_path)
        .with_context(|| format!("Failed to parse manifest: {manifest_path}"))?;
    validate_provisioners(&manifest).context("Manifest validation failed: provisioners")?;
    check_text_file_modes(&manifest.nodes).context("Manifest validation failed: text files")?;

    let tailscale_auth_key = resolve_tailscale_key(&manifest, server_url, config)?;

    // Read ztp_config, text_files and scripts relative to the manifest, and $VAR values.
    resolve_manifest_files(&mut manifest, manifest_path)?;

    // Container secret files go outside the manifest, so the server never stores them.
    let manifest_dir = Path::new(manifest_path).parent().unwrap_or(Path::new("."));
    let node_secrets = container_secrets(&manifest, manifest_dir)?;

    // Where the lab is started from, stored with it and shown by `sherpa inspect`.
    let client_host = gethostname().to_string_lossy().into_owned();
    let client_host = (!client_host.is_empty()).then_some(client_host);
    let client_folder = get_cwd().context("Failed to read the current folder")?;

    // Serialize manifest to JSON for transmission
    let manifest_value =
        serde_json::to_value(&manifest).context("Failed to serialize manifest to JSON")?;

    // Limits the connect only: the operation streams its progress for as long as it runs.
    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
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

    // Create RPC request with authentication token
    let up_request = RpcRequest::new(
        "up",
        serde_json::json!({
            "lab_id": lab_id,
            "manifest": manifest_value,
            "tailscale_auth_key": tailscale_auth_key,
            "node_secrets": node_secrets,
            "client_host": client_host,
            "client_folder": client_folder,
            "token": token,
        }),
    );

    println!("Starting lab creation...\n");

    // Call streaming RPC with progress callback
    let target = OperationTarget {
        operation: StoppableOperation::Up,
        lab_id,
        token: &token,
        server_url,
        server_connection: &config.server_connection,
    };
    let up_response =
        match call_streaming_or_interrupt(&mut rpc_client, up_request, print_up_progress, &target)
            .await
            .context("Up RPC call failed")?
        {
            Waited::Response(response) => response,
            Waited::Interrupted => bail!("Lab creation interrupted"),
        };

    // Close connection
    rpc_client.close().await.ok();

    // Handle errors
    let up_data: UpResponse = up_response.into_result("Lab creation failed")?;

    // The lab exists now, so a file that cannot be saved is only a warning.
    match save_lab_files(&client_folder, &LabFiles::from(&up_data)) {
        Ok(ssh_config) => register_ssh_config(&ssh_config),
        Err(e) => println!(
            "\n{} Warning: Not all lab files were saved: {e:#}\n   Run: sherpa download {lab_id} --force",
            Emoji::Warning
        ),
    }

    // Display results
    display_up_results(&up_data);

    // ssh provisioners run from the client, so their files never pass through the server.
    let ready_nodes = ssh_ready_nodes(&up_data.nodes);
    let provisioned = provision_lab(
        &manifest,
        manifest_dir,
        lab_id,
        Some(&ready_nodes),
        None,
        container_results(&node_secrets),
        &HostKeyRefresh::new(lab_id, server_url, config, manifest_dir),
    )
    .await;

    // A partly created lab is the failure to report: its missing nodes are why they were not
    // provisioned.
    if !up_data.success {
        bail!(
            "Lab {}-{} was only partly created; see the errors above",
            up_data.lab_info.name,
            up_data.lab_info.id
        );
    }

    provisioned?;
    Ok(())
}

/// The nodes that came up with SSH available.
fn ssh_ready_nodes(nodes: &[NodeInfo]) -> Vec<String> {
    let mut ready = Vec::new();
    for node in nodes {
        if node.ssh_port.is_some() {
            ready.push(node.name.clone());
        }
    }
    ready
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

        let resolved_path = resolve_client_path(manifest_dir, &ztp_path_str);

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

/// Read each node's `startup_scripts` and `user_scripts`, base64-encode them, and fill in
/// `startup_scripts_data` and `user_scripts_data` for the server. Paths may start with `~`;
/// relative paths start at the manifest's folder.
pub(crate) fn resolve_scripts(
    manifest: &mut topology::Manifest,
    manifest_path: &str,
) -> Result<()> {
    let manifest_dir = Path::new(manifest_path).parent().unwrap_or(Path::new("."));
    for node in &mut manifest.nodes {
        if let Some(paths) = node
            .startup_scripts
            .as_deref()
            .filter(|paths| !paths.is_empty())
        {
            node.startup_scripts_data = Some(read_scripts(
                &node.name,
                paths,
                "startup_script",
                manifest_dir,
            )?);
        }
        if let Some(paths) = node
            .user_scripts
            .as_deref()
            .filter(|paths| !paths.is_empty())
        {
            node.user_scripts_data = Some(read_scripts(
                &node.name,
                paths,
                "user_script",
                manifest_dir,
            )?);
        }
    }
    Ok(())
}

/// Read and encode the scripts at `paths` for node `node`; `kind` names them in errors.
fn read_scripts(
    node: &str,
    paths: &[String],
    kind: &str,
    manifest_dir: &Path,
) -> Result<Vec<StartupScript>> {
    let mut scripts = Vec::with_capacity(paths.len());
    for path in paths {
        let resolved = resolve_client_path(manifest_dir, path);
        let contents = fs::read_to_string(&resolved).with_context(|| {
            format!(
                "Failed to read {kind} for node '{node}': {}",
                resolved.display()
            )
        })?;
        if contents.is_empty() {
            bail!("{kind} for node '{node}' is empty: {}", resolved.display());
        }
        let filename = resolved
            .file_name()
            .map_or_else(|| path.clone(), |name| name.to_string_lossy().to_string());
        scripts.push(StartupScript {
            filename,
            content: base64_encode(&contents),
        });
    }
    Ok(scripts)
}

/// Resolve every client file and `$VAR` value the server needs, for `up` and `redeploy`.
pub(crate) fn resolve_manifest_files(
    manifest: &mut topology::Manifest,
    manifest_path: &str,
) -> Result<()> {
    resolve_ztp_configs(manifest, manifest_path)?;
    resolve_text_files(manifest, manifest_path)?;
    resolve_scripts(manifest, manifest_path)?;
    resolve_environment_variables(manifest, client_variable)
}

/// Resolve `text_files` file paths relative to the manifest directory.
/// Reads each file, base64-encodes the contents, and populates
/// `text_files_data` for transmission to the server.
pub(crate) fn resolve_text_files(
    manifest: &mut topology::Manifest,
    manifest_path: &str,
) -> Result<()> {
    let manifest_dir = Path::new(manifest_path).parent().unwrap_or(Path::new("."));

    for node in &mut manifest.nodes {
        let text_files = match &node.text_files {
            Some(f) if !f.is_empty() => f.clone(),
            _ => continue,
        };

        let mut file_data = Vec::with_capacity(text_files.len());
        for file in &text_files {
            let resolved_path = resolve_client_path(manifest_dir, &file.src);

            let contents = load_file(&resolved_path.to_string_lossy())
                .with_context(|| format!("Failed to read text_file for node '{}'", node.name))?;

            file_data.push(topology::TextFileData {
                content: base64_encode(&contents),
                dst: file.dst.clone(),
                user: file.user.clone(),
                group: file.group.clone(),
                permissions: file.permissions,
            });
        }

        node.text_files_data = Some(file_data);
    }

    Ok(())
}

/// Resolve `$ENV_VAR` references in environment variable values.
///
/// For each node's `environment_variables`, entries of the form `KEY=$VAR`
/// have the `$VAR` portion resolved with `lookup`, from the user's shell environment.
/// Only exact `$VAR` values are expanded (not partial references like `prefix$VAR`).
pub(crate) fn resolve_environment_variables(
    manifest: &mut topology::Manifest,
    lookup: fn(&str) -> Option<String>,
) -> Result<()> {
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
                let resolved = lookup(var_name).with_context(|| {
                    format!(
                        "Environment variable '{}' referenced in node '{}' is not set",
                        var_name, node.name
                    )
                })?;
                *entry = format!("{key}={resolved}");
            }
        }
    }

    Ok(())
}

/// Show a progress message `up` streams: a status line, under its phase's heading when it
/// starts one, or a server log line.
fn print_up_progress(msg_text: &str) {
    if let Ok(status_msg) = serde_json::from_str::<StatusMessage>(msg_text) {
        if status_msg.r#type == "status" {
            if let Some(phase) = &status_msg.phase {
                println!();
                term_msg_underline(phase);
            }
            println!("{} {}", status_emoji(status_msg.kind), status_msg.message);
        }
    } else if let Ok(log_msg) = serde_json::from_str::<LogMessage>(msg_text)
        && log_msg.r#type == "log"
    {
        println!("[LOG] {}", log_msg.message);
    }
}

/// Display lab creation results
fn display_up_results(response: &UpResponse) {
    println!();
    term_msg_surround("Lab Creation Results");

    // Lab Info
    let lab_info_table = render_lab_info_table(&response.lab_info);
    println!("{lab_info_table}");

    if let Some(tailscale) = &response.tailscale {
        println!("{}", render_tailnet_table(tailscale));
    }

    // Node information
    if !response.nodes.is_empty() {
        println!(
            "\n{}",
            render_nodes_table(&response.nodes, &response.lab_info.id)
        );
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{NodeKind, NodeModel, NodeState};
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
    fn startup_and_user_scripts_are_read_from_the_manifest_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("boot.sh"), "echo boot").unwrap();
        fs::write(dir.path().join("user.sh"), "echo user").unwrap();
        let mut manifest = topology::Manifest {
            nodes: vec![topology::Node {
                name: "dev01".into(),
                startup_scripts: Some(vec!["boot.sh".into()]),
                user_scripts: Some(vec!["user.sh".into()]),
                ..Default::default()
            }],
            ..Default::default()
        };
        let manifest_path = dir.path().join("manifest.toml");

        resolve_scripts(&mut manifest, manifest_path.to_str().unwrap()).unwrap();

        let startup = manifest.nodes[0].startup_scripts_data.as_ref().unwrap();
        assert_eq!(startup[0].filename, "boot.sh");
        assert_eq!(startup[0].content, base64_encode("echo boot"));
        let user = manifest.nodes[0].user_scripts_data.as_ref().unwrap();
        assert_eq!(user[0].filename, "user.sh");
        assert_eq!(user[0].content, base64_encode("echo user"));
    }

    #[test]
    fn an_empty_script_is_refused_naming_its_node() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("empty.sh"), "").unwrap();
        let mut manifest = topology::Manifest {
            nodes: vec![topology::Node {
                name: "dev01".into(),
                user_scripts: Some(vec!["empty.sh".into()]),
                ..Default::default()
            }],
            ..Default::default()
        };
        let manifest_path = dir.path().join("manifest.toml");

        let error = resolve_scripts(&mut manifest, manifest_path.to_str().unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("user_script for node 'dev01' is empty"),
            "{error}"
        );
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
        let err_msg = format!("{}", result.expect_err("should be error"));
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

    /// A client environment with only `SHERPA_TEST_TOKEN` set.
    fn test_token_lookup(name: &str) -> Option<String> {
        (name == "SHERPA_TEST_TOKEN").then(|| "secret123".to_owned())
    }

    /// A client environment with no variables set.
    fn no_variables(_name: &str) -> Option<String> {
        None
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

        resolve_environment_variables(&mut manifest, no_variables).expect("should succeed");

        let env_vars = manifest.nodes[0]
            .environment_variables
            .as_ref()
            .expect("should have env vars");
        assert_eq!(env_vars[0], "EDITOR=vim");
        assert_eq!(env_vars[1], "LANG=en_US.UTF-8");
    }

    #[test]
    fn test_resolve_environment_variables_env_ref() {
        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            environment_variables: Some(vec!["MY_TOKEN=$SHERPA_TEST_TOKEN".to_string()]),
            ..Default::default()
        }]);

        resolve_environment_variables(&mut manifest, test_token_lookup).expect("should succeed");

        let env_vars = manifest.nodes[0]
            .environment_variables
            .as_ref()
            .expect("should have env vars");
        assert_eq!(env_vars[0], "MY_TOKEN=secret123");
    }

    #[test]
    fn test_resolve_environment_variables_missing_env_var() {
        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            environment_variables: Some(vec!["MY_VAR=$SHERPA_NONEXISTENT_VAR_12345".to_string()]),
            ..Default::default()
        }]);

        let result = resolve_environment_variables(&mut manifest, no_variables);
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

        resolve_environment_variables(&mut manifest, no_variables)
            .expect("should succeed with no env vars");
    }

    #[test]
    fn test_resolve_text_files_base64_encodes_content() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let file_path = dir.path().join("config.json");
        std::fs::write(&file_path, r#"{"key": "value"}"#).expect("write file");

        let manifest_path = dir.path().join("manifest.toml");

        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            text_files: Some(vec![topology::TextFile {
                src: file_path.to_str().expect("path").to_string(),
                dst: "/etc/app/config.json".to_string(),
                user: "root".to_string(),
                group: "root".to_string(),
                permissions: 644,
            }]),
            ..Default::default()
        }]);

        resolve_text_files(&mut manifest, manifest_path.to_str().expect("path"))
            .expect("resolve should succeed");

        let data = manifest.nodes[0]
            .text_files_data
            .as_ref()
            .expect("should have text_files_data");
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].dst, "/etc/app/config.json");
        assert_eq!(data[0].user, "root");
        assert_eq!(data[0].group, "root");
        assert_eq!(data[0].permissions, 644);

        let decoded = base64_decode(&data[0].content).expect("should decode");
        assert_eq!(decoded, r#"{"key": "value"}"#);
    }

    fn node_info(name: &str, ssh_port: Option<u16>) -> NodeInfo {
        NodeInfo {
            name: name.to_string(),
            kind: NodeKind::VirtualMachine,
            model: NodeModel::UbuntuLinux,
            status: NodeState::Running,
            ip_address: None,
            ssh_port,
        }
    }

    #[test]
    fn provisioners_run_on_nodes_that_came_up_with_ssh() {
        let nodes = vec![node_info("dev01", Some(22)), node_info("dev02", None)];
        assert_eq!(ssh_ready_nodes(&nodes), vec!["dev01"]);
    }

    #[test]
    fn test_resolve_manifest_files_resolves_text_files_and_scripts() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        std::fs::write(dir.path().join("my_file.txt"), "hello world").expect("write file");
        std::fs::write(dir.path().join("setup.sh"), "#!/bin/sh\n").expect("write script");
        let manifest_path = dir.path().join("manifest.toml");

        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            text_files: Some(vec![topology::TextFile {
                src: "my_file.txt".to_string(),
                dst: "/opt/data/my_file.txt".to_string(),
                user: "sherpa".to_string(),
                group: "sherpa".to_string(),
                permissions: 644,
            }]),
            user_scripts: Some(vec!["setup.sh".to_string()]),
            ..Default::default()
        }]);

        resolve_manifest_files(&mut manifest, manifest_path.to_str().expect("path"))
            .expect("resolve should succeed");

        let node = &manifest.nodes[0];
        let text_files = node.text_files_data.as_ref().expect("text_files_data");
        assert_eq!(
            base64_decode(&text_files[0].content).expect("decode"),
            "hello world"
        );
        assert!(node.user_scripts_data.is_some());
    }

    #[test]
    fn test_resolve_text_files_relative_path() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let file_path = dir.path().join("my_file.txt");
        std::fs::write(&file_path, "hello world").expect("write file");

        let manifest_path = dir.path().join("manifest.toml");

        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            text_files: Some(vec![topology::TextFile {
                src: "my_file.txt".to_string(),
                dst: "/opt/data/my_file.txt".to_string(),
                user: "sherpa".to_string(),
                group: "sherpa".to_string(),
                permissions: 755,
            }]),
            ..Default::default()
        }]);

        resolve_text_files(&mut manifest, manifest_path.to_str().expect("path"))
            .expect("resolve should succeed");

        let data = manifest.nodes[0]
            .text_files_data
            .as_ref()
            .expect("should have text_files_data");
        let decoded = base64_decode(&data[0].content).expect("should decode");
        assert_eq!(decoded, "hello world");
    }

    #[test]
    fn test_resolve_text_files_missing_file_error() {
        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            text_files: Some(vec![topology::TextFile {
                src: "/nonexistent/path/file.txt".to_string(),
                dst: "/etc/file.txt".to_string(),
                user: "root".to_string(),
                group: "root".to_string(),
                permissions: 644,
            }]),
            ..Default::default()
        }]);

        let result = resolve_text_files(&mut manifest, "/tmp/manifest.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_text_files_skips_nodes_without_text_files() {
        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            ..Default::default()
        }]);

        resolve_text_files(&mut manifest, "/tmp/manifest.toml").expect("resolve should succeed");

        assert!(manifest.nodes[0].text_files_data.is_none());
    }

    #[test]
    fn test_resolve_environment_variables_invalid_format() {
        let mut manifest = test_manifest(vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            environment_variables: Some(vec!["NO_EQUALS_SIGN".to_string()]),
            ..Default::default()
        }]);

        let result = resolve_environment_variables(&mut manifest, no_variables);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("KEY=VALUE"));
    }
}
