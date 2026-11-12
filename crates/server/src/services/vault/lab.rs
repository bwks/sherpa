//! Vault nodes in the lab lifecycle. Sherpa initialises each Vault node that runs its generated
//! config, keeps the unseal key and root token with the lab's files, and unseals it.

use std::ffi::OsStr;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use shared::data::{NodeModel, StatusKind, VaultInfo};
use shared::konst::{READINESS_SLEEP, SHERPA_LABS_PATH};

use super::api::{SealStatus, VaultClient};
use crate::services::progress::ProgressSender;
use crate::tls::generator::write_file_with_permissions;

/// Directory in the lab directory holding each Vault node's credentials.
const VAULT_DIR: &str = "vault";
/// How often to check whether an unsealed Vault is active; it takes a second or two.
const ACTIVE_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Init requests sent to a Vault node in one operation. Each after the first is sent only once
/// Vault has confirmed it is still not initialised.
const INIT_ATTEMPTS: u32 = 2;

/// A Vault node's credentials, saved as `<lab>/vault/<node>.toml`, readable only by the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultCredentials {
    pub address: String,
    pub unseal_key: String,
    pub root_token: String,
}

/// What to do after an init request failed, decided by [`after_failed_init`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterFailedInit {
    /// Vault is not initialised: send init again.
    Retry,
    /// Vault is not initialised and every init attempt failed.
    GiveUp,
    /// Vault is initialised: the request initialised it, but its response, the only copy of
    /// the unseal key and root token, never reached Sherpa.
    LostInitResponse,
}

impl VaultCredentials {
    fn into_info(self, name: &str) -> VaultInfo {
        VaultInfo {
            name: name.to_owned(),
            address: self.address,
            root_token: self.root_token,
            unseal_key: self.unseal_key,
        }
    }
}

/// Whether Sherpa initialises and unseals a node: a Vault node running the config Sherpa
/// generates. A user's own config may not serve the API over TLS, so it is left to the user.
pub fn sherpa_initialises_vault(model: NodeModel, own_config: bool) -> bool {
    model == NodeModel::HashicorpVault && !own_config
}

/// The lab directory where Sherpa keeps its Vault nodes' credentials.
pub fn vault_dir(lab_id: &str) -> PathBuf {
    Path::new(SHERPA_LABS_PATH).join(lab_id).join(VAULT_DIR)
}

fn credentials_path(dir: &Path, node_name: &str) -> PathBuf {
    dir.join(format!("{node_name}.toml"))
}

fn save_credentials(dir: &Path, node_name: &str, credentials: &VaultCredentials) -> Result<()> {
    let content = toml::to_string(credentials).context("Failed to serialize Vault credentials")?;
    write_file_with_permissions(&credentials_path(dir, node_name), content.as_bytes(), 0o600)
        .with_context(|| format!("Failed to save the credentials of Vault node '{node_name}'"))
}

fn load_credentials(dir: &Path, node_name: &str) -> Result<Option<VaultCredentials>> {
    let path = credentials_path(dir, node_name);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let credentials =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(Some(credentials))
}

/// Wait for a Vault node's API, initialise it if it is new, unseal it if it is sealed, and wait
/// until it is active.
///
/// A new Vault is initialised with one key share. Its unseal key and root token are saved
/// before it is unsealed, so they are never lost to a failure in between.
#[instrument(skip(lab_ca_pem, progress), fields(%lab_id, %node_name, %node_address))]
pub async fn ensure_vault_unsealed(
    lab_id: &str,
    node_name: &str,
    node_address: Ipv4Addr,
    lab_ca_pem: &str,
    timeout: Duration,
    progress: &ProgressSender,
) -> Result<VaultInfo> {
    let client = VaultClient::new(node_address, lab_ca_pem)?;
    let credentials =
        unseal_with_client(&client, &vault_dir(lab_id), node_name, timeout, progress).await?;
    Ok(credentials.into_info(node_name))
}

/// [`ensure_vault_unsealed`] on a Vault client, keeping the credentials in `dir`.
#[instrument(level = "debug", skip(client, progress), fields(address = %client.address(), dir = %dir.display()))]
async fn unseal_with_client(
    client: &VaultClient,
    dir: &Path,
    node_name: &str,
    timeout: Duration,
    progress: &ProgressSender,
) -> Result<VaultCredentials> {
    let status = wait_for_api(client, node_name, timeout).await?;
    // Initialised before Sherpa sent any init request: Sherpa saved its credentials when it
    // initialised it, or it was initialised outside Sherpa.
    let (credentials, sealed) = if status.initialized {
        let saved = load_credentials(dir, node_name)?;
        (saved_credentials(node_name, saved)?, status.sealed)
    } else {
        // A newly initialised Vault is sealed.
        (
            initialise(client, dir, node_name, timeout, progress).await?,
            true,
        )
    };

    if sealed {
        let status = client
            .unseal(&credentials.unseal_key)
            .await
            .with_context(|| format!("Failed to unseal Vault node '{node_name}'"))?;
        if status.sealed {
            bail!("Vault node '{node_name}' is still sealed after its unseal key was applied");
        }
        let _ = progress.send_status(format!("Vault {node_name} - unsealed"), StatusKind::Done);
    }
    wait_until_active(client, node_name, timeout).await?;
    Ok(credentials)
}

/// The saved credentials of a Vault node that was already initialised when Sherpa first read
/// its seal status. With none saved, Sherpa did not initialise it.
fn saved_credentials(node_name: &str, saved: Option<VaultCredentials>) -> Result<VaultCredentials> {
    saved.with_context(|| {
        format!(
            "Vault node '{node_name}' was initialised outside Sherpa, so Sherpa has no unseal key or root token for it"
        )
    })
}

/// Initialise a new Vault and save its credentials.
///
/// An init request that fails may still complete inside Vault, which hands out the unseal key
/// and root token in the response only. So after a failed request the seal status is read
/// again first, and init is sent again only when Vault confirms it is not initialised.
async fn initialise(
    client: &VaultClient,
    dir: &Path,
    node_name: &str,
    timeout: Duration,
    progress: &ProgressSender,
) -> Result<VaultCredentials> {
    let mut failed_inits = 0;
    loop {
        let init_error = match client.init().await {
            Ok(init) => {
                let unseal_key =
                    init.keys_base64.into_iter().next().with_context(|| {
                        format!("Vault node '{node_name}' returned no unseal key")
                    })?;
                let credentials = VaultCredentials {
                    address: client.address().to_owned(),
                    unseal_key,
                    root_token: init.root_token,
                };
                save_credentials(dir, node_name, &credentials)?;
                let _ = progress
                    .send_status(format!("Vault {node_name} - initialised"), StatusKind::Done);
                return Ok(credentials);
            }
            Err(error) => error,
        };
        failed_inits += 1;
        let status = match wait_for_api(client, node_name, timeout).await {
            Ok(status) => status,
            Err(status_error) => {
                return Err(status_unknown_error(node_name, &init_error, &status_error));
            }
        };
        match after_failed_init(status, failed_inits) {
            AfterFailedInit::Retry => {
                tracing::warn!(
                    %node_name,
                    error = %format!("{init_error:#}"),
                    "Vault init failed and Vault is not initialised; sending init again"
                );
            }
            AfterFailedInit::LostInitResponse => {
                return Err(lost_init_response_error(node_name, &init_error));
            }
            AfterFailedInit::GiveUp => {
                return Err(init_error.context(format!(
                    "Failed to initialise Vault node '{node_name}' in {failed_inits} attempts; it is not initialised"
                )));
            }
        }
    }
}

/// What to do after an init request of this operation failed, from the seal status read
/// right after it and the number of init requests that have failed.
fn after_failed_init(status: SealStatus, failed_inits: u32) -> AfterFailedInit {
    if status.initialized {
        AfterFailedInit::LostInitResponse
    } else if failed_inits < INIT_ATTEMPTS {
        AfterFailedInit::Retry
    } else {
        AfterFailedInit::GiveUp
    }
}

/// How to recover a Vault node whose credentials Sherpa does not have.
fn reset_instructions(node_name: &str) -> String {
    format!(
        "Sherpa has not reset it. To recover, reset the node's Vault data with \
         `sherpa redeploy --node {node_name}` in the lab directory: the node starts again with \
         empty Vault storage, and Sherpa initialises and unseals it and saves its new credentials. \
         Anything already written to this Vault is lost"
    )
}

fn lost_init_response_error(node_name: &str, init_error: &anyhow::Error) -> anyhow::Error {
    anyhow!(
        "Vault node '{node_name}' is initialised, but Sherpa lost the init response carrying its \
         unseal key and root token: the init request failed ({init_error:#}) after Vault had \
         started it. Vault gives these out only once, so Sherpa cannot unseal the node. {}",
        reset_instructions(node_name)
    )
}

fn status_unknown_error(
    node_name: &str,
    init_error: &anyhow::Error,
    status_error: &anyhow::Error,
) -> anyhow::Error {
    anyhow!(
        "Failed to initialise Vault node '{node_name}' ({init_error:#}), and its seal status \
         could not be read afterwards ({status_error:#}), so Sherpa cannot tell whether the init \
         completed. If it did, Sherpa lost the init response carrying the node's unseal key and \
         root token. {}",
        reset_instructions(node_name)
    )
}

/// Poll until the node is active, within `timeout`. Right after an unseal, Raft storage elects
/// a leader first, and requests other than status answer 500 until it has.
async fn wait_until_active(client: &VaultClient, node_name: &str, timeout: Duration) -> Result<()> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        match client.is_active().await {
            Ok(true) => return Ok(()),
            Ok(false) => tracing::debug!(%node_name, "Vault not active yet"),
            Err(error) => {
                tracing::debug!(%node_name, error = %format!("{error:#}"), "Vault health not answered");
            }
        }
        tokio::time::sleep(ACTIVE_POLL_INTERVAL).await;
    }
    bail!(
        "Vault node '{node_name}' did not become active within {}s",
        timeout.as_secs()
    )
}

/// Poll the seal status until the API answers, within `timeout`.
async fn wait_for_api(
    client: &VaultClient,
    node_name: &str,
    timeout: Duration,
) -> Result<SealStatus> {
    let started = Instant::now();
    loop {
        match client.seal_status().await {
            Ok(status) => return Ok(status),
            Err(error) if started.elapsed() < timeout => {
                tracing::debug!(%node_name, error = %format!("{error:#}"), "Vault API not ready yet");
                tokio::time::sleep(Duration::from_secs(READINESS_SLEEP)).await;
            }
            Err(error) => {
                return Err(error.context(format!(
                    "Vault node '{node_name}' API did not answer within {}s",
                    timeout.as_secs()
                )));
            }
        }
    }
}

/// The credentials of every Vault node in a lab's Vault directory (see [`vault_dir`]), for
/// `sherpa download`.
#[instrument(level = "debug", fields(dir = %dir.display()))]
pub fn read_vault_credentials(dir: &Path) -> Result<Vec<VaultInfo>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut vaults = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to list {}", dir.display()))? {
        let path = entry?.path();
        if path.extension() != Some(OsStr::new("toml")) {
            continue;
        }
        let Some(node_name) = path.file_stem().and_then(OsStr::to_str) else {
            continue;
        };
        if let Some(credentials) = load_credentials(dir, node_name)? {
            vaults.push(credentials.into_info(node_name));
        }
    }
    vaults.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(vaults)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::net::IpAddr;
    use std::os::unix::fs::PermissionsExt;

    use bollard::Docker;
    use bollard::models::{ContainerCreateBody, HostConfig};
    use bollard::query_parameters::{
        CreateContainerOptions, InspectContainerOptions, RemoveContainerOptions,
        StartContainerOptions,
    };
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    use crate::tls::generator::{
        CaSpec, KeyAlgorithm, ServerSubject, generate_ca, issue_server_certificate,
    };

    /// The Vault image the integration tests run, as on a lab node.
    const TEST_VAULT_IMAGE: &str = "hashicorp/vault:1.21";
    /// Where a test Vault finds its config and certificate.
    const TEST_VAULT_DIR: &str = "/sherpa-test";
    /// How long a test waits for its Vault, as a lab's readiness timeout.
    const TEST_TIMEOUT: Duration = Duration::from_secs(120);

    const NOT_INITIALISED: SealStatus = SealStatus {
        initialized: false,
        sealed: true,
    };
    const INITIALISED: SealStatus = SealStatus {
        initialized: true,
        sealed: true,
    };

    /// A Vault container running Sherpa's Raft and TLS config, removed when dropped. It waits
    /// for its certificate, issued for the address Docker gives it, before Vault starts.
    struct TestVault {
        docker: Docker,
        name: String,
        address: Ipv4Addr,
        lab_ca: String,
        _files: TempDir,
    }

    impl TestVault {
        async fn start(name: &str) -> TestVault {
            let docker = Docker::connect_with_local_defaults().unwrap();
            let files = TempDir::new().unwrap();
            let config = format!(
                "storage \"raft\" {{\n  path = \"/vault/data\"\n  node_id = \"{name}\"\n}}\n\
                 listener \"tcp\" {{\n  address = \"0.0.0.0:8200\"\n  \
                 tls_cert_file = \"{TEST_VAULT_DIR}/node.crt\"\n  \
                 tls_key_file = \"{TEST_VAULT_DIR}/node.key\"\n}}\n\
                 disable_mlock = true\napi_addr = \"https://127.0.0.1:8200\"\n\
                 cluster_addr = \"https://127.0.0.1:8201\"\n"
            );
            fs::write(files.path().join("config.hcl"), config).unwrap();
            let command = format!(
                "until [ -f {TEST_VAULT_DIR}/ready ]; do sleep 0.1; done; mkdir -p /vault/data \
                 && exec vault server -config={TEST_VAULT_DIR}/config.hcl"
            );
            let _ = docker
                .remove_container(
                    name,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;
            docker
                .create_container(
                    Some(CreateContainerOptions {
                        name: Some(name.to_owned()),
                        ..Default::default()
                    }),
                    ContainerCreateBody {
                        image: Some(TEST_VAULT_IMAGE.to_owned()),
                        cmd: Some(vec!["sh".to_owned(), "-c".to_owned(), command]),
                        host_config: Some(HostConfig {
                            binds: Some(vec![format!(
                                "{}:{TEST_VAULT_DIR}",
                                files.path().display()
                            )]),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            docker
                .start_container(name, None::<StartContainerOptions>)
                .await
                .unwrap();
            let details = docker
                .inspect_container(name, None::<InspectContainerOptions>)
                .await
                .unwrap();
            let networks: HashMap<_, _> = details.network_settings.unwrap().networks.unwrap();
            let address: Ipv4Addr = networks
                .values()
                .find_map(|network| network.ip_address.clone().filter(|ip| !ip.is_empty()))
                .unwrap()
                .parse()
                .unwrap();

            let ca_spec = CaSpec {
                common_name: Some("Vault test CA"),
                organization: Some("Sherpa"),
                key_algorithm: KeyAlgorithm::EcdsaP256,
            };
            let ca = generate_ca(&ca_spec, 1).unwrap();
            let node = issue_server_certificate(
                &ca_spec,
                &ca.key,
                ServerSubject {
                    common_name: name,
                    organization: "Sherpa",
                    dns_names: &[],
                    ip_addresses: &[IpAddr::V4(address)],
                },
                KeyAlgorithm::EcdsaP256,
                1,
            )
            .unwrap();
            fs::write(files.path().join("node.crt"), &node.cert).unwrap();
            fs::write(files.path().join("node.key"), &node.key).unwrap();
            fs::write(files.path().join("ready"), "").unwrap();
            TestVault {
                docker,
                name: name.to_owned(),
                address,
                lab_ca: ca.cert,
                _files: files,
            }
        }

        fn client(&self) -> VaultClient {
            VaultClient::new(self.address, &self.lab_ca).unwrap()
        }

        async fn remove(self) {
            self.docker
                .remove_container(
                    &self.name,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
        }
    }

    fn progress() -> ProgressSender {
        let (tx, _rx) = mpsc::unbounded_channel();
        ProgressSender::new(tx)
    }

    fn credentials(token: &str) -> VaultCredentials {
        VaultCredentials {
            address: "https://172.31.1.21:8200".to_owned(),
            unseal_key: "oQ==".to_owned(),
            root_token: token.to_owned(),
        }
    }

    #[test]
    fn only_vault_nodes_with_the_generated_config_are_initialised() {
        assert!(sherpa_initialises_vault(NodeModel::HashicorpVault, false));
        assert!(!sherpa_initialises_vault(NodeModel::HashicorpVault, true));
        assert!(!sherpa_initialises_vault(NodeModel::AristaCeos, false));
    }

    #[test]
    fn credentials_are_saved_owner_only_and_read_back() {
        let dir = TempDir::new().unwrap();
        save_credentials(dir.path(), "vault01", &credentials("hvs.one")).unwrap();
        let path = credentials_path(dir.path(), "vault01");
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            load_credentials(dir.path(), "vault01").unwrap(),
            Some(credentials("hvs.one"))
        );
        assert_eq!(load_credentials(dir.path(), "vault02").unwrap(), None);
    }

    #[test]
    fn download_lists_each_vault_by_name() {
        let dir = TempDir::new().unwrap();
        save_credentials(dir.path(), "vault02", &credentials("hvs.two")).unwrap();
        save_credentials(dir.path(), "vault01", &credentials("hvs.one")).unwrap();
        fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let vaults = read_vault_credentials(dir.path()).unwrap();
        let names: Vec<&str> = vaults.iter().map(|vault| vault.name.as_str()).collect();
        assert_eq!(names, ["vault01", "vault02"]);
        assert_eq!(vaults[1].root_token, "hvs.two");
        assert_eq!(
            read_vault_credentials(&dir.path().join("missing")).unwrap(),
            [] as [shared::data::VaultInfo; 0]
        );
    }

    #[test]
    fn init_is_sent_again_only_while_vault_is_not_initialised() {
        assert_eq!(
            after_failed_init(NOT_INITIALISED, 1),
            AfterFailedInit::Retry
        );
        assert_eq!(
            after_failed_init(NOT_INITIALISED, INIT_ATTEMPTS),
            AfterFailedInit::GiveUp
        );
    }

    #[test]
    fn a_vault_initialised_after_a_failed_init_lost_its_init_response() {
        for failed_inits in 1..=INIT_ATTEMPTS {
            assert_eq!(
                after_failed_init(INITIALISED, failed_inits),
                AfterFailedInit::LostInitResponse
            );
            let unsealed = SealStatus {
                initialized: true,
                sealed: false,
            };
            assert_eq!(
                after_failed_init(unsealed, failed_inits),
                AfterFailedInit::LostInitResponse
            );
        }
    }

    #[test]
    fn a_vault_initialised_before_sherpa_sent_init_uses_its_saved_credentials() {
        assert_eq!(
            saved_credentials("vault01", Some(credentials("hvs.one"))).unwrap(),
            credentials("hvs.one")
        );
        let message = format!("{:#}", saved_credentials("vault01", None).unwrap_err());
        assert!(message.contains("initialised outside Sherpa"), "{message}");
    }

    #[test]
    fn a_lost_init_response_says_so_and_how_to_reset_the_node() {
        let init_error = anyhow!("operation timed out").context("Vault API did not answer");
        let message = format!("{:#}", lost_init_response_error("vault01", &init_error));
        assert!(
            message.contains("Sherpa lost the init response"),
            "{message}"
        );
        assert!(
            message.contains("Vault API did not answer: operation timed out"),
            "{message}"
        );
        assert!(message.contains("Sherpa has not reset it"), "{message}");
        assert!(
            message.contains("`sherpa redeploy --node vault01`"),
            "{message}"
        );
        assert!(!message.contains("outside Sherpa"), "{message}");
    }

    #[test]
    fn an_unknown_status_after_a_failed_init_says_how_to_reset_the_node() {
        let message = format!(
            "{:#}",
            status_unknown_error(
                "vault01",
                &anyhow!("operation timed out"),
                &anyhow!("connection refused")
            )
        );
        assert!(message.contains("operation timed out"), "{message}");
        assert!(message.contains("connection refused"), "{message}");
        assert!(
            message.contains("cannot tell whether the init"),
            "{message}"
        );
        assert!(
            message.contains("`sherpa redeploy --node vault01`"),
            "{message}"
        );
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon and the hashicorp/vault:1.21 image"]
    async fn a_new_vault_is_initialised_then_unsealed_with_its_saved_credentials() {
        let vault = TestVault::start("sherpa-test-vault-init").await;
        let dir = TempDir::new().unwrap();
        let client = vault.client();
        let first = unseal_with_client(&client, dir.path(), "vault01", TEST_TIMEOUT, &progress())
            .await
            .unwrap();
        assert_eq!(
            load_credentials(dir.path(), "vault01").unwrap(),
            Some(first.clone())
        );
        let again = unseal_with_client(&client, dir.path(), "vault01", TEST_TIMEOUT, &progress())
            .await
            .unwrap();
        assert_eq!(again, first);

        let elsewhere = TempDir::new().unwrap();
        let error = unseal_with_client(
            &client,
            elsewhere.path(),
            "vault01",
            TEST_TIMEOUT,
            &progress(),
        )
        .await
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("initialised outside Sherpa"),
            "{error:#}"
        );
        vault.remove().await;
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon and the hashicorp/vault:1.21 image"]
    async fn a_timed_out_init_that_completes_in_vault_is_reported_as_a_lost_init_response() {
        let name = "sherpa-test-vault-lost-init";
        let dir = TempDir::new().unwrap();
        // Saved for the node's earlier Vault, as before a redeploy: they must not be used.
        save_credentials(dir.path(), "vault01", &credentials("hvs.stale")).unwrap();

        // Raft init takes seconds, so the request times out and Vault finishes it anyway.
        let vault = TestVault::start(name).await;
        let client = vault.client().with_init_timeout(Duration::from_millis(200));
        let error = unseal_with_client(&client, dir.path(), "vault01", TEST_TIMEOUT, &progress())
            .await
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("Sherpa lost the init response"),
            "{message}"
        );
        assert!(!message.contains("outside Sherpa"), "{message}");
        assert_eq!(
            load_credentials(dir.path(), "vault01").unwrap(),
            Some(credentials("hvs.stale"))
        );
        // Mid-init, Vault answers the seal status with a 500, so poll it as Sherpa does.
        let status = wait_for_api(&vault.client(), "vault01", TEST_TIMEOUT)
            .await
            .unwrap();
        assert!(status.initialized);
        vault.remove().await;

        // The reset the error asks for: the node again with empty Vault storage.
        let vault = TestVault::start(name).await;
        let fresh = unseal_with_client(
            &vault.client(),
            dir.path(),
            "vault01",
            TEST_TIMEOUT,
            &progress(),
        )
        .await
        .unwrap();
        assert_ne!(fresh.root_token, "hvs.stale");
        assert_eq!(
            load_credentials(dir.path(), "vault01").unwrap(),
            Some(fresh)
        );
        vault.remove().await;
    }
}
