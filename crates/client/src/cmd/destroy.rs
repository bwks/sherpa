use anyhow::{Context, Result};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use shared::data::{ClientConfig, DestroyResponse, InspectResponse, NodeInfo};
use shared::konst::{
    LAB_FILE_NAME, SHERPA_KNOWN_HOSTS_FILE, SHERPA_LAB_CA_FILE, SHERPA_MANIFEST_FILE,
    SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_FILE,
};
use shared::util::{
    Emoji, confirm, display_destroy_results, get_cwd, remove_lab_ssh_include,
    render_lab_info_table, render_nodes_table, term_msg_surround,
};
use topology::Manifest;

use super::outcome::ensure_lab_destroyed;
use super::progress::print_status;
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Destroy lab  to sherpad server
///
/// Flow:
/// 1. Inspect lab to show what will be destroyed
/// 2. Ask for user confirmation
/// 3. If confirmed, destroy lab and show detailed results
#[tracing::instrument(skip_all, fields(%lab_name, %lab_id))]
pub async fn destroy(
    lab_name: &str,
    lab_id: &str,
    server_url: &str,
    config: &ClientConfig,
    yes: bool,
) -> Result<()> {
    term_msg_surround(&format!("Destroy environment - {lab_name}-{lab_id}"));

    let token = load_token()?;

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
    let inspect_data: InspectResponse =
        inspect_response.into_result("Failed to inspect lab before destroy")?;

    show_lab_to_destroy(&inspect_data);

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
        .call_streaming(destroy_request, print_status)
        .await
        .context("Destroy RPC call failed")?;

    // Close connection
    rpc_client.close().await.ok();

    // Handle destroy errors
    let destroy_data: DestroyResponse = destroy_response.into_result("Destroy operation failed")?;

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

    remove_local_lab_state(&inspect_data);

    ensure_lab_destroyed(&destroy_data, "sherpa destroy")
}

/// Show the lab about to be destroyed: its details and its nodes.
fn show_lab_to_destroy(inspect_data: &InspectResponse) {
    // Display lab details
    println!("{}", render_lab_info_table(&inspect_data.lab_info));

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
            ssh_port: None,
        })
        .collect();

    if !nodes.is_empty() {
        println!(
            "\n{}",
            render_nodes_table(&nodes, &inspect_data.lab_info.id)
        );
    }
}

/// Remove the lab's Include from `~/.ssh/sherpa_lab_hosts` and the files saved for it in the
/// current folder. Failures only warn: the lab itself is gone.
fn remove_local_lab_state(inspect_data: &InspectResponse) {
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
    println!();
    confirm(&format!(
        "Are you sure you want to destroy lab {lab_name}-{lab_id} ({device_count} devices)?"
    ))
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
