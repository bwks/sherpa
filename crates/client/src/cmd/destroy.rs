use anyhow::{Context, Result, bail};
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use shared::data::{
    ClientConfig, DestroyResponse, InspectResponse, NodeInfo, StatusKind, StatusMessage,
};
use shared::error::RpcErrorCode;
use shared::konst::{
    LAB_FILE_NAME, SHERPA_KNOWN_HOSTS_FILE, SHERPA_LAB_CA_FILE, SHERPA_MANIFEST_FILE,
    SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_FILE,
};
use shared::util::{
    Emoji, display_destroy_results, get_cwd, get_username, remove_lab_ssh_include,
    render_lab_info_table, render_nodes_table, term_msg_surround,
};
use topology::Manifest;

use super::outcome::ensure_lab_destroyed;
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Destroy lab  to sherpad server
///
/// Flow:
/// 1. Inspect lab to show what will be destroyed
/// 2. Ask for user confirmation
/// 3. If confirmed, destroy lab and show detailed results
pub async fn destroy(
    lab_name: &str,
    lab_id: &str,
    server_url: &str,
    config: &ClientConfig,
    yes: bool,
) -> Result<()> {
    term_msg_surround(&format!("Destroy environment - {lab_name}-{lab_id}"));

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

    // Phase 1: Inspect lab to show what will be destroyed
    println!("\nFetching lab details...\n");

    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    // Connect and inspect
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to sherpad server")?;

    // Get current username (for display only - server uses token)
    let _username = get_username()?;

    let inspect_request = RpcRequest::new(
        "inspect",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token.clone(),
        }),
    );

    let inspect_response = rpc_client
        .call(inspect_request)
        .await
        .context("Inspect RPC call failed")?;

    // Handle inspect errors
    if let Some(error) = inspect_response.error {
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

        bail!("Failed to inspect lab before destroy");
    }

    let inspect_result = inspect_response
        .result
        .context("No result in inspect response")?;
    let inspect_data: InspectResponse =
        serde_json::from_value(inspect_result).context("Failed to parse inspect response")?;

    // Display lab details
    let lab_info_table = render_lab_info_table(&inspect_data.lab_info);
    println!("{}", lab_info_table);

    // Convert DeviceInfo to NodeInfo for table rendering
    let nodes: Vec<NodeInfo> = inspect_data
        .devices
        .iter()
        .map(|device| NodeInfo {
            name: device.name.clone(),
            kind: device.kind.to_string(),
            model: device.model,
            status: device.state,
            ip_address: if device.mgmt_ipv4.is_empty() {
                None
            } else {
                Some(device.mgmt_ipv4.clone())
            },
            ssh_port: Some(22), // Default SSH port
        })
        .collect();

    if !nodes.is_empty() {
        println!(
            "\n{}",
            render_nodes_table(&nodes, &inspect_data.lab_info.id)
        );
    }

    let device_count = inspect_data.devices.len();

    // Phase 2: Ask for confirmation (skip if --yes flag is set)
    if !yes && !confirm_destroy(lab_name, lab_id, device_count)? {
        println!("\n{} Destroy operation cancelled by user", Emoji::Warning);
        return Ok(());
    }

    // Phase 3: Destroy lab
    println!("\n{} Destroying lab resources...\n", Emoji::Warning);

    let destroy_request = RpcRequest::new(
        "destroy",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );

    let destroy_response = rpc_client
        .call_streaming(destroy_request, |msg_text| {
            if let Ok(status_msg) = serde_json::from_str::<StatusMessage>(msg_text)
                && status_msg.r#type == "status"
            {
                let emoji = match status_msg.kind {
                    StatusKind::Progress => Emoji::Progress,
                    StatusKind::Done => Emoji::Success,
                    StatusKind::Info => Emoji::Info,
                    StatusKind::Waiting => Emoji::Hourglass,
                };
                println!("{} {}", emoji, status_msg.message);
            }
        })
        .await
        .context("Destroy RPC call failed")?;

    // Close connection
    rpc_client.close().await.ok();

    // Handle destroy errors
    if let Some(error) = destroy_response.error {
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

        bail!("Destroy operation failed");
    }

    let destroy_result = destroy_response
        .result
        .context("No result in destroy response")?;
    let destroy_data: DestroyResponse =
        serde_json::from_value(destroy_result).context("Failed to parse destroy response")?;

    // Display detailed results
    display_destroy_results(&destroy_data)?;

    // While the lab still exists on the server, keep the local SSH files: they still reach any
    // nodes that survived, and the next `sherpa destroy` removes them.
    if !destroy_data.summary.database_records_deleted {
        println!(
            "\n{} Local SSH files kept: the lab still exists on the server",
            Emoji::Warning
        );
        return ensure_lab_destroyed(&destroy_data, "sherpa destroy");
    }

    // Remove this lab's Include from ~/.ssh/sherpa_lab_hosts, then the files saved for it.
    match get_cwd() {
        Ok(cwd) => {
            let dir = Path::new(&cwd);
            let ssh_config = dir.join(SHERPA_SSH_CONFIG_FILE);
            match remove_lab_ssh_include(&ssh_config.to_string_lossy()) {
                Ok(()) => {
                    println!(
                        "\n{} SSH config removed from ~/.ssh/sherpa_lab_hosts",
                        Emoji::Success
                    );
                }
                Err(e) => {
                    println!(
                        "\n{} Warning: Failed to clean up ~/.ssh/sherpa_lab_hosts: {}",
                        Emoji::Warning,
                        e
                    );
                }
            }
            let devices: Vec<String> = inspect_data
                .devices
                .iter()
                .map(|device| device.name.clone())
                .chain(inspect_data.inactive_devices.iter().cloned())
                .collect();
            let clusters = manifest_cluster_names(dir);
            remove_local_lab_files(&local_lab_files(dir, &devices, &clusters));
        }
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {}",
                Emoji::Warning,
                e
            );
        }
    }

    ensure_lab_destroyed(&destroy_data, "sherpa destroy")
}

/// The files `sherpa up` and `sherpa download` save in a lab's folder `dir`: its SSH access,
/// lab CA and `lab-info.toml`, and the Vault and Keycloak credentials of its `devices` and the
/// credentials of its Talos `clusters`, which are named after their node or cluster.
fn local_lab_files(dir: &Path, devices: &[String], clusters: &[String]) -> Vec<PathBuf> {
    let fixed = [
        SHERPA_SSH_CONFIG_FILE,
        SHERPA_SSH_PRIVATE_KEY_FILE,
        SHERPA_KNOWN_HOSTS_FILE,
        SHERPA_LAB_CA_FILE,
        LAB_FILE_NAME,
    ]
    .map(ToString::to_string);
    let credentials = devices.iter().flat_map(|device| {
        [
            format!("vault-{device}.toml"),
            format!("keycloak-{device}.toml"),
        ]
    });
    let cluster_files = clusters.iter().flat_map(|cluster| {
        [
            format!("kubeconfig-{cluster}"),
            format!("talosconfig-{cluster}"),
        ]
    });
    fixed
        .into_iter()
        .chain(credentials)
        .chain(cluster_files)
        .map(|name| dir.join(name))
        .collect()
}

/// The Talos cluster names of the manifest in `dir`, or none when it has no manifest that loads.
fn manifest_cluster_names(dir: &Path) -> Vec<String> {
    let path = dir.join(SHERPA_MANIFEST_FILE);
    if !path.exists() {
        return Vec::new();
    }
    match Manifest::load_file(&path.to_string_lossy()) {
        Ok(manifest) => manifest
            .talos_clusters
            .unwrap_or_default()
            .into_iter()
            .map(|cluster| cluster.name)
            .collect(),
        Err(e) => {
            println!(
                "\n{} Warning: Kubernetes credentials kept: {} does not load: {e:#}",
                Emoji::Warning,
                path.display()
            );
            Vec::new()
        }
    }
}

/// Delete each of `files` that exists, reporting each one deleted or failed.
fn remove_local_lab_files(files: &[PathBuf]) {
    for path in files {
        match fs::remove_file(path) {
            Ok(()) => println!("{} Deleted {}", Emoji::Success, path.display()),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                tracing::debug!(path = %path.display(), "No local lab file to delete");
            }
            Err(e) => println!(
                "{} Warning: Failed to delete {}: {e}",
                Emoji::Warning,
                path.display()
            ),
        }
    }
}

/// Ask user for confirmation before destroying lab
fn confirm_destroy(lab_name: &str, lab_id: &str, device_count: usize) -> Result<bool> {
    println!(
        "\n{} WARNING: This will permanently destroy all lab resources!",
        Emoji::Warning
    );
    print!(
        "\nAre you sure you want to destroy lab {}-{} ({} devices)? [y/N]: ",
        lab_name, lab_id, device_count
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let input = input.trim().to_lowercase();
    Ok(input == "y" || input == "yes")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn destroy_removes_only_the_files_sherpa_saved_for_the_lab() {
        let dir = tempfile::tempdir().unwrap();
        let saved = [
            "sherpa_ssh_config",
            "sherpa_ssh_key",
            "sherpa_known_hosts",
            "lab-ca.crt",
            "lab-info.toml",
            "vault-vault01.toml",
            "keycloak-kc01.toml",
            "kubeconfig-k8s",
            "talosconfig-k8s",
        ];
        let kept = ["manifest.toml", "kubeconfig-prod", "vault-notes.txt"];
        for name in saved.iter().chain(&kept) {
            fs::write(dir.path().join(name), "x").unwrap();
        }

        let devices = ["vault01".to_string(), "kc01".to_string()];
        remove_local_lab_files(&local_lab_files(dir.path(), &devices, &["k8s".to_string()]));

        let left: BTreeSet<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(left, kept.iter().map(ToString::to_string).collect());
    }
}
