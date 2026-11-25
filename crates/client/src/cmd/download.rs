use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use shared::data::{ClientConfig, DownloadLabResponse};
use shared::util::{Emoji, LabFolder, get_cwd, term_msg_surround};

use crate::error::ClientError;
use crate::lab_files::{LabFileSet, LabFiles, register_ssh_config, save_lab_files};
use crate::token::load_token;
use crate::ws_client::{RpcRequest, WebSocketClient};

/// Download lab files from server and write to the current directory.
#[tracing::instrument(skip_all, fields(%lab_id, force))]
pub async fn download(
    lab_id: &str,
    force: bool,
    server_url: &str,
    config: &ClientConfig,
) -> Result<()> {
    term_msg_surround(&format!("Download Lab Files - {lab_id}"));

    let cwd = get_cwd().context("Failed to determine working directory")?;
    let folder = LabFolder::new(Path::new(&cwd));

    let token = load_token()?;

    let timeout = Duration::from_secs(config.server_connection.timeout_secs);
    let ws_client = WebSocketClient::new(
        server_url.to_string(),
        timeout,
        config.server_connection.clone(),
    );

    println!("Connecting to server: {}", server_url);
    let mut rpc_client = ws_client
        .connect()
        .await
        .context("Failed to connect to server")?;

    let request = RpcRequest::new(
        "download",
        serde_json::json!({
            "lab_id": lab_id,
            "token": token,
        }),
    );

    println!("Requesting lab files from server...");
    let response = rpc_client.call(request).await.context("RPC call failed")?;
    rpc_client.close().await.ok();

    let data: DownloadLabResponse = response.into_result("Download failed")?;

    let files = LabFileSet::new(&folder, &LabFiles::from(&data))?;
    if !force {
        refuse_existing_lab_files(&folder, &files)?;
    }
    let ssh_config = save_lab_files(&folder, &files)?;
    register_ssh_config(&ssh_config);

    println!("\n{} Lab files downloaded successfully", Emoji::Success);

    Ok(())
}

/// Refuse to write `files` in `folder` when any of them is there already, naming the first
/// one found, by its path in the folder.
fn refuse_existing_lab_files(folder: &LabFolder, files: &LabFileSet) -> Result<()> {
    if let Some(path) = files.first_existing() {
        bail!(ClientError::LabFilesExist {
            path: path.strip_prefix(folder.root()).unwrap_or(path).to_owned()
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use shared::data::{LabInfo, VaultInfo};

    use super::*;

    const LAB_INFO: &str = r#"id = "fecb1a12"
name = "sherpa-dev"
user = "dev"
ipv4_network = "172.31.3.0/24"
ipv4_gateway = "172.31.3.1"
ipv4_router = "172.31.3.2"
"#;

    #[test]
    fn saved_lab_files_010_download_refuses_a_folder_with_only_a_credential_file() {
        let dir = tempfile::tempdir().unwrap();
        let folder = LabFolder::new(dir.path());
        let lab_info: LabInfo = LAB_INFO.parse().unwrap();
        let vaults = [VaultInfo {
            name: "vault01".to_owned(),
            address: "https://172.31.3.21:8200".to_owned(),
            root_token: "hvs.root".to_owned(),
            unseal_key: "oQ==".to_owned(),
        }];
        let files = LabFileSet::new(
            &folder,
            &LabFiles {
                ssh_config: "",
                known_hosts: "",
                lab_info: &lab_info,
                lab_ca: "",
                vaults: &vaults,
                keycloaks: &[],
                surreal_dbs: &[],
                clusters: &[],
            },
        )
        .unwrap();
        fs::create_dir_all(folder.vault_dir()).unwrap();
        fs::write(folder.vault("vault01"), "kept").unwrap();

        let error = refuse_existing_lab_files(&folder, &files).unwrap_err();

        let vault_file = Path::new(".sherpa")
            .join("vault")
            .join("vault-vault01.toml");
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::LabFilesExist { path }) if *path == vault_file
            ),
            "{error:#}"
        );
        assert!(!folder.lab_info().exists());
        assert_eq!(fs::read_to_string(folder.vault("vault01")).unwrap(), "kept");
    }
}
