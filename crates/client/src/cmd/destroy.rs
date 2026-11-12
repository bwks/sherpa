use anyhow::{Context, Result, bail};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use shared::data::{ClientConfig, DestroyResponse, InspectResponse, NodeInfo};
use shared::konst::SHERPA_SSH_CONFIG_FILE;
use shared::util::{
    Emoji, LabFolder, confirm, display_destroy_results, get_cwd, get_user_ssh_dir,
    remove_lab_ssh_include_at, term_msg_surround,
};

use super::outcome::ensure_lab_destroyed;
use super::progress::print_status;
use crate::lab_folder_migration::legacy_lab_files;
use crate::table::{render_lab_info_table, render_nodes_table};
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

    remove_local_lab_state();

    ensure_lab_destroyed(&destroy_data, "sherpa destroy")
}

/// Show the lab about to be destroyed: its details and its nodes.
fn show_lab_to_destroy(inspect_data: &InspectResponse) {
    // Display lab details
    println!("{}", render_lab_info_table(&inspect_data.lab_info));
    if let Err(not_ready) = inspect_data.lab_phase.ensure_ready() {
        println!("{} Not ready: {not_ready}", Emoji::Warning);
    }

    // Convert DeviceInfo to NodeInfo for table rendering
    let nodes: Vec<NodeInfo> = inspect_data
        .devices
        .iter()
        .map(|device| NodeInfo {
            name: device.name.clone(),
            kind: device.kind.clone(),
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

/// Remove the lab's Include from `~/.ssh/sherpa_lab_hosts` and its files from the current
/// folder. Failures only warn: the lab itself is gone.
fn remove_local_lab_state() {
    let cwd = match get_cwd() {
        Ok(cwd) => cwd,
        Err(e) => {
            println!(
                "\n{} Warning: Could not determine working directory: {e}",
                Emoji::Warning
            );
            return;
        }
    };
    let ssh_dir = match get_user_ssh_dir() {
        Ok(ssh_dir) => ssh_dir,
        Err(e) => {
            println!(
                "\n{} Warning: Failed to clean up ~/.ssh/sherpa_lab_hosts: {e:#}",
                Emoji::Warning
            );
            return;
        }
    };
    println!();
    for warning in remove_lab_files(Path::new(&cwd), &ssh_dir) {
        println!("{} Warning: {warning}", Emoji::Warning);
    }
}

/// Remove `lab_dir`'s lab from `ssh_dir/sherpa_lab_hosts` and delete its local files; the
/// warnings for what could not be done.
///
/// This works whether or not the folder was migrated to `.sherpa` (a failed migration does not
/// stop destroy): the Include of either the new `.sherpa/ssh/config` or the old
/// `sherpa_ssh_config` is removed, `.sherpa` is deleted whole, and the files an older client left
/// loose are deleted by their exact generated names. Nothing else in the folder is touched.
#[tracing::instrument(level = "debug", fields(lab_dir = %lab_dir.display(), ssh_dir = %ssh_dir.display()))]
fn remove_lab_files(lab_dir: &Path, ssh_dir: &Path) -> Vec<String> {
    let folder = LabFolder::new(lab_dir);
    let mut warnings = Vec::new();
    let includes = [folder.ssh_config(), lab_dir.join(SHERPA_SSH_CONFIG_FILE)];
    let mut include_failed = false;
    for include in includes {
        if let Err(e) = remove_lab_ssh_include_at(ssh_dir, &include.to_string_lossy()) {
            include_failed = true;
            warnings.push(format!("Failed to clean up ~/.ssh/sherpa_lab_hosts: {e:#}"));
        }
    }
    if !include_failed {
        println!(
            "{} SSH config removed from ~/.ssh/sherpa_lab_hosts",
            Emoji::Success
        );
    }
    match remove_state_dir(&folder) {
        Ok(true) => println!(
            "{} Deleted {}",
            Emoji::Success,
            folder.state_dir().display()
        ),
        Ok(false) => tracing::debug!("No .sherpa folder to delete"),
        Err(e) => warnings.push(format!("{e:#}")),
    }
    for file in legacy_lab_files(lab_dir) {
        match fs::remove_file(&file) {
            Ok(()) => println!("{} Deleted {}", Emoji::Success, file.display()),
            Err(e) => warnings.push(format!("Failed to delete {}: {e}", file.display())),
        }
    }
    warnings
}

/// Delete `folder`'s `.sherpa` folder and everything in it, and nothing else; whether there was
/// one. A symlink at `.sherpa` is refused, so nothing it points to is deleted.
#[tracing::instrument(level = "debug", fields(folder = %folder.root().display()))]
fn remove_state_dir(folder: &LabFolder) -> Result<bool> {
    let state_dir = folder.state_dir();
    match fs::symlink_metadata(&state_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!(
            "{} is a symlink, so it was not deleted",
            state_dir.display()
        ),
        Ok(metadata) if !metadata.is_dir() => {
            bail!(
                "{} is not a folder, so it was not deleted",
                state_dir.display()
            )
        }
        Ok(_) => {
            fs::remove_dir_all(&state_dir)
                .with_context(|| format!("Failed to delete {}", state_dir.display()))?;
            Ok(true)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("Failed to read {}", state_dir.display())),
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
    use crate::lab_folder_migration::migrate_lab_folder;

    fn names(dir: &Path) -> BTreeSet<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    #[test]
    fn destroy_removes_the_sherpa_folder_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        for path in [
            folder.ssh_config(),
            folder.ssh_key(),
            folder.lab_info(),
            folder.lab_ca(),
            folder.vault("vault01"),
            folder.kubeconfig("k8s"),
            // A file Sherpa did not write, such as the dev lab's direct SSH config.
            folder.ssh_dir().join("direct_config"),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        let kept = ["manifest.toml", "kubeconfig-prod", "lab-notes.txt"];
        for name in kept {
            fs::write(dir.path().join(name), "x").unwrap();
        }

        assert!(remove_state_dir(&folder).unwrap());

        assert_eq!(names(dir.path()), kept.map(ToString::to_string).into());
        assert!(
            !remove_state_dir(&folder).unwrap(),
            "nothing left to delete"
        );
    }

    /// An old-layout lab folder registered in `ssh_dir`, with a `.sherpa` folder whose `ssh`
    /// is a file, so migrating it fails.
    fn conflicting_old_layout(lab: &Path, ssh_dir: &Path) {
        for (name, content) in [
            ("sherpa_ssh_config", "Host dev\n"),
            ("sherpa_ssh_key", "k"),
            ("sherpa_known_hosts", "h"),
            ("lab-info.toml", "x"),
            ("manifest.toml", "mine"),
            ("notes.txt", "mine"),
        ] {
            fs::write(lab.join(name), content).unwrap();
        }
        fs::create_dir_all(lab.join(".sherpa")).unwrap();
        fs::write(lab.join(".sherpa/ssh"), "in the way").unwrap();
        fs::create_dir_all(ssh_dir).unwrap();
        fs::write(
            ssh_dir.join("sherpa_lab_hosts"),
            format!(
                "Include {}\nInclude /other/lab/.sherpa/ssh/config\n",
                lab.join("sherpa_ssh_config").display()
            ),
        )
        .unwrap();
    }

    #[test]
    fn destroy_cleans_up_after_a_failed_migration() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        conflicting_old_layout(lab.path(), &ssh_dir);
        assert!(migrate_lab_folder(lab.path(), &ssh_dir).is_err());

        let warnings = remove_lab_files(lab.path(), &ssh_dir);

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            names(lab.path()),
            ["manifest.toml", "notes.txt"]
                .map(ToString::to_string)
                .into()
        );
        assert_eq!(
            fs::read_to_string(ssh_dir.join("sherpa_lab_hosts")).unwrap(),
            "Include /other/lab/.sherpa/ssh/config\n"
        );
    }

    #[test]
    fn destroy_removes_the_new_include_too() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        fs::create_dir_all(&ssh_dir).unwrap();
        let config = LabFolder::new(lab.path()).ssh_config();
        fs::write(
            ssh_dir.join("sherpa_lab_hosts"),
            format!("Include {}\nInclude /other/config\n", config.display()),
        )
        .unwrap();

        let warnings = remove_lab_files(lab.path(), &ssh_dir);
        assert!(warnings.is_empty(), "{warnings:?}");

        assert_eq!(
            fs::read_to_string(ssh_dir.join("sherpa_lab_hosts")).unwrap(),
            "Include /other/config\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn destroy_after_a_failed_migration_still_refuses_a_symlinked_sherpa_folder() {
        let lab = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::write(elsewhere.path().join("precious"), "x").unwrap();
        let ssh_dir = home.path().join(".ssh");
        fs::write(lab.path().join("sherpa_ssh_key"), "k").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), lab.path().join(".sherpa")).unwrap();
        assert!(migrate_lab_folder(lab.path(), &ssh_dir).is_err());

        let warnings = remove_lab_files(lab.path(), &ssh_dir);

        assert!(
            warnings.iter().any(|w| w.contains("symlink")),
            "{warnings:?}"
        );
        assert!(elsewhere.path().join("precious").is_file());
        assert!(fs::symlink_metadata(lab.path().join(".sherpa")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn destroy_refuses_a_symlinked_sherpa_folder() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::write(elsewhere.path().join("precious"), "x").unwrap();
        let link = dir.path().join(".sherpa");
        std::os::unix::fs::symlink(elsewhere.path(), &link).unwrap();

        let error = remove_state_dir(&LabFolder::new(dir.path())).unwrap_err();

        assert!(error.to_string().contains("symlink"), "{error}");
        assert!(elsewhere.path().join("precious").is_file());
        assert!(fs::symlink_metadata(&link).is_ok());
    }
}
