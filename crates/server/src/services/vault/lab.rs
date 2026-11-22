//! Vault nodes in the lab lifecycle. Sherpa initialises each Vault node that runs its generated
//! config, keeps the unseal key and root token with the lab's files, and unseals it: in `up` and
//! redeploy, after resume starts it again, and whenever the scanner finds it sealed.

use std::ffi::OsStr;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use topology::Manifest;
use tracing::instrument;

use shared::data::{NodeModel, StatusKind, VaultInfo, VaultSealed};
use shared::konst::SHERPA_LABS_PATH;
use shared::util::write_file_with_permissions;

use super::api::{SealStatus, VaultClient, VaultRefusal};
use crate::daemon::vault_seals::VaultSeals;
use crate::konst::READINESS_SLEEP;
use crate::services::progress::ProgressSender;

/// Directory in the lab directory holding each Vault node's credentials.
pub(crate) const VAULT_DIR: &str = "vault";
/// How often to check whether an unsealed Vault is active; it takes a second or two.
const ACTIVE_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Init requests sent to a Vault node in one operation. Each after the first is sent only once
/// Vault has confirmed it is still not initialised.
const INIT_ATTEMPTS: u32 = 2;

/// Why the saved unseal key did not unseal a Vault, decided by [`apply_unseal_key`].
#[derive(Debug)]
enum UnsealFailure {
    /// Vault refused the key, or took it and stayed sealed.
    KeyRefused(anyhow::Error),
    /// The request failed otherwise: it may be tried again.
    NotApplied(anyhow::Error),
}

/// What Sherpa found of a Vault node it manages, after unsealing it with its saved key where it
/// was sealed. See [`unseal_with_saved_key`].
#[derive(Debug)]
pub enum SealOutcome {
    /// Vault is not initialised. Only `up` and redeploy initialise one, so nothing was done.
    NotInitialised,
    /// Vault was not sealed.
    Unsealed,
    /// Vault was sealed, and its saved key unsealed it now.
    UnsealedNow,
    /// Vault is sealed, and Sherpa cannot unseal it.
    Sealed(VaultSealed),
    /// Vault is sealed, and the unseal request failed for a reason other than the key, such as
    /// Vault not answering. It is tried again on the next scan.
    NotApplied(anyhow::Error),
}

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

/// Whether node `node_name` of the lab whose stored definition is `definition` is a Vault node
/// Sherpa manages (see [`sherpa_initialises_vault`]). A Vault node with its own `ztp.config` is
/// the user's, and Sherpa never touches it; a node the definition does not have is not managed.
pub fn manages_vault(definition: &Manifest, node_name: &str) -> bool {
    definition.nodes.iter().any(|node| {
        node.name == node_name && sherpa_initialises_vault(node.model, node.ztp.config.is_some())
    })
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
    // A TOML error quotes the line it is on, which may hold the unseal key or root token: only
    // its message is kept.
    let credentials = toml::from_str(&content)
        .map_err(|error| anyhow!("not a Vault credentials file: {}", error.message()))
        .with_context(|| format!("Failed to parse {}", path.display()))?;
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
        apply_unseal_key(client, node_name, &credentials.unseal_key)
            .await
            .map_err(
                |(UnsealFailure::KeyRefused(error) | UnsealFailure::NotApplied(error))| error,
            )?;
        let _ = progress.send_status(format!("Vault {node_name} - unsealed"), StatusKind::Done);
    }
    wait_until_active(client, node_name, timeout).await?;
    Ok(credentials)
}

/// Unseal a sealed Vault with its unseal key: the one unseal step of `up`, redeploy, resume and
/// the scanner.
async fn apply_unseal_key(
    client: &VaultClient,
    node_name: &str,
    unseal_key: &str,
) -> Result<(), UnsealFailure> {
    match client.unseal(unseal_key).await {
        Ok(status) if !status.sealed => Ok(()),
        Ok(_) => Err(UnsealFailure::KeyRefused(anyhow!(
            "Vault node '{node_name}' is still sealed after its unseal key was applied"
        ))),
        Err(error) => {
            let refused = error
                .downcast_ref::<VaultRefusal>()
                .is_some_and(VaultRefusal::refused_request);
            let error = error.context(format!("Failed to unseal Vault node '{node_name}'"));
            Err(if refused {
                UnsealFailure::KeyRefused(error)
            } else {
                UnsealFailure::NotApplied(error)
            })
        }
    }
}

/// Unseal a Vault node Sherpa manages with the unseal key saved in `dir`, if `status`, read
/// from it just before, says it is sealed. A Vault that is not initialised is left alone: only
/// `up` and redeploy initialise one. Nothing is sent when no key is saved or the key file cannot
/// be read.
#[instrument(level = "debug", skip(client), fields(address = %client.address(), dir = %dir.display()))]
pub async fn unseal_with_saved_key(
    client: &VaultClient,
    dir: &Path,
    node_name: &str,
    status: SealStatus,
) -> SealOutcome {
    if !status.initialized {
        return SealOutcome::NotInitialised;
    }
    if !status.sealed {
        return SealOutcome::Unsealed;
    }
    let credentials = match load_credentials(dir, node_name) {
        Ok(Some(credentials)) => credentials,
        Ok(None) => return SealOutcome::Sealed(VaultSealed::NoSavedKey),
        Err(error) => {
            return SealOutcome::Sealed(VaultSealed::KeyFileUnreadable {
                cause: error.root_cause().to_string(),
            });
        }
    };
    match apply_unseal_key(client, node_name, &credentials.unseal_key).await {
        Ok(()) => SealOutcome::UnsealedNow,
        Err(UnsealFailure::KeyRefused(error)) => {
            tracing::debug!(%node_name, error = %format!("{error:#}"), "Vault refused the saved unseal key");
            SealOutcome::Sealed(VaultSealed::KeyRefused)
        }
        Err(UnsealFailure::NotApplied(error)) => SealOutcome::NotApplied(error),
    }
}

/// After resume started a Vault node Sherpa manages: wait for its API, unseal it with its saved
/// key as `up` does, and wait until it is active, within `timeout`.
///
/// # Errors
///
/// Fails when the API does not answer, or an unsealed Vault does not become active, in time.
#[instrument(skip(lab_ca_pem), fields(%lab_id, %node_name, %node_address))]
pub async fn unseal_started_vault(
    lab_id: &str,
    node_name: &str,
    node_address: Ipv4Addr,
    lab_ca_pem: &str,
    timeout: Duration,
) -> Result<SealOutcome> {
    let client = VaultClient::new(node_address, lab_ca_pem)?;
    unseal_started_with_client(&client, &vault_dir(lab_id), node_name, timeout).await
}

/// [`unseal_started_vault`] on a Vault client, with the credentials in `dir`.
async fn unseal_started_with_client(
    client: &VaultClient,
    dir: &Path,
    node_name: &str,
    timeout: Duration,
) -> Result<SealOutcome> {
    let status = wait_for_api(client, node_name, timeout).await?;
    let outcome = unseal_with_saved_key(client, dir, node_name, status).await;
    if matches!(outcome, SealOutcome::Unsealed | SealOutcome::UnsealedNow) {
        wait_until_active(client, node_name, timeout).await?;
    }
    Ok(outcome)
}

/// Keep what `outcome` says of Vault node `node_name` of lab `lab_id` in `seals`, for `inspect`,
/// and log a change: at warn when the node is found sealed with a reason it was not recorded
/// with, and at info when it is unsealed or found unsealed after being recorded sealed. An
/// outcome that decides nothing (`NotApplied`) keeps what was recorded.
pub fn record_seal_outcome(
    seals: &VaultSeals,
    lab_id: &str,
    node_name: &str,
    outcome: &SealOutcome,
) {
    match outcome {
        SealOutcome::Sealed(sealed) => {
            let previous = seals.record(lab_id, node_name, Some(sealed.clone()));
            if previous.as_ref() != Some(sealed) {
                tracing::warn!(%lab_id, %node_name, state = %sealed, "Vault node is sealed, and Sherpa cannot unseal it");
            }
        }
        SealOutcome::UnsealedNow => {
            let _ = seals.record(lab_id, node_name, None);
            tracing::info!(%lab_id, %node_name, "Unsealed Vault node with its saved unseal key");
        }
        SealOutcome::Unsealed | SealOutcome::NotInitialised => {
            if seals.record(lab_id, node_name, None).is_some() {
                tracing::info!(%lab_id, %node_name, "Vault node is no longer sealed");
            }
        }
        SealOutcome::NotApplied(error) => {
            tracing::debug!(%lab_id, %node_name, error = %format!("{error:#}"), "Vault unseal not applied; trying again on the next scan");
        }
    }
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
pub(crate) mod tests {
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
    use shared::data::{KeycloakInfo, SurrealDbInfo};
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    use crate::log_capture::capture_log;
    use crate::services::keycloak::lab::{ensure_keycloak_credentials, read_keycloak_credentials};
    use crate::services::surreal_db::{ensure_surreal_db_credentials, read_surreal_db_credentials};

    use crate::tls::generator::{
        CaSpec, KeyAlgorithm, ServerSubject, generate_ca, issue_server_certificate,
    };

    /// The Vault image the integration tests run, as on a lab node.
    pub(crate) const TEST_VAULT_IMAGE: &str = "hashicorp/vault:1.21";
    /// Where a test Vault finds its config and certificate.
    const TEST_VAULT_DIR: &str = "/sherpa-test";
    /// How long a test waits for its Vault, as a lab's readiness timeout.
    pub(crate) const TEST_TIMEOUT: Duration = Duration::from_secs(120);

    const NOT_INITIALISED: SealStatus = SealStatus {
        initialized: false,
        sealed: true,
    };
    const INITIALISED: SealStatus = SealStatus {
        initialized: true,
        sealed: true,
    };

    /// A sealed seal status of an initialised Vault.
    const SEALED: SealStatus = SealStatus {
        initialized: true,
        sealed: true,
    };

    /// A Vault container running Sherpa's Raft and TLS config, removed by `remove`. It waits
    /// for its certificate, issued for the address Docker gives it, before Vault starts.
    pub(crate) struct TestVault {
        docker: Docker,
        name: String,
        address: Ipv4Addr,
        lab_ca: String,
        _files: TempDir,
    }

    impl TestVault {
        pub(crate) async fn start(name: &str) -> TestVault {
            let docker = Docker::connect_with_local_defaults().unwrap();
            let files = TempDir::new().unwrap();
            let config = format!(
                r#"storage "raft" {{
  path = "/vault/data"
  node_id = "{name}"
}}
listener "tcp" {{
  address = "0.0.0.0:8200"
  tls_cert_file = "{TEST_VAULT_DIR}/node.crt"
  tls_key_file = "{TEST_VAULT_DIR}/node.key"
}}
disable_mlock = true
api_addr = "https://127.0.0.1:8200"
cluster_addr = "https://127.0.0.1:8201"
"#
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

        pub(crate) fn client(&self) -> VaultClient {
            VaultClient::new(self.address, &self.lab_ca).unwrap()
        }

        pub(crate) async fn remove(self) {
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

    /// A Vault Sherpa initialised, its credentials saved in the returned folder, then sealed
    /// again, as a restart leaves it.
    pub(crate) async fn sealed_test_vault(name: &str) -> (TestVault, TempDir) {
        let vault = TestVault::start(name).await;
        let dir = TempDir::new().unwrap();
        let credentials = unseal_with_client(
            &vault.client(),
            dir.path(),
            "vault01",
            TEST_TIMEOUT,
            &progress(),
        )
        .await
        .unwrap();
        vault
            .client()
            .write(&credentials.root_token, "sys/seal", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(vault.client().seal_status().await.unwrap().sealed);
        (vault, dir)
    }

    /// A client for a Vault that is not there: any request it sends fails.
    pub(crate) fn absent_vault() -> VaultClient {
        let ca = generate_ca(
            &CaSpec {
                common_name: Some("Vault test CA"),
                organization: Some("Sherpa"),
                key_algorithm: KeyAlgorithm::EcdsaP256,
            },
            1,
        )
        .unwrap();
        VaultClient::new(Ipv4Addr::LOCALHOST, &ca.cert).unwrap()
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
    fn vault_init_006_only_vault_nodes_with_the_generated_config_are_initialised() {
        assert!(sherpa_initialises_vault(NodeModel::HashicorpVault, false));
        assert!(!sherpa_initialises_vault(NodeModel::HashicorpVault, true));
        assert!(!sherpa_initialises_vault(NodeModel::AristaEos, false));
    }

    #[test]
    fn lab_credentials_018_a_vault_s_token_and_key_are_saved_for_sherpad_and_read_back() {
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
    fn lab_credentials_019_each_node_s_credentials_are_listed_by_name_in_name_order() {
        let lab = TempDir::new().unwrap();
        let vaults = lab.path().join("vault");
        let keycloaks = lab.path().join("keycloak");
        let surreal_dbs = lab.path().join("surrealdb");
        fs::create_dir_all(&vaults).unwrap();
        save_credentials(&vaults, "vault02", &credentials("hvs.two")).unwrap();
        save_credentials(&vaults, "vault01", &credentials("hvs.one")).unwrap();
        ensure_keycloak_credentials(&keycloaks, "kc02", "https://172.31.1.24:8443", "alice")
            .unwrap();
        let kc01 =
            ensure_keycloak_credentials(&keycloaks, "kc01", "https://172.31.1.23:8443", "alice")
                .unwrap();
        ensure_surreal_db_credentials(&surreal_dbs, "db02", "http://172.31.1.26:8000").unwrap();
        let db01 =
            ensure_surreal_db_credentials(&surreal_dbs, "db01", "http://172.31.1.25:8000").unwrap();
        for dir in [&vaults, &keycloaks, &surreal_dbs] {
            fs::write(dir.join("notes.txt"), "ignored").unwrap();
        }

        let vault_list = read_vault_credentials(&vaults).unwrap();
        let keycloak_list = read_keycloak_credentials(&keycloaks).unwrap();
        let surreal_db_list = read_surreal_db_credentials(&surreal_dbs).unwrap();

        let vault_names: Vec<&str> = vault_list.iter().map(|vault| vault.name.as_str()).collect();
        assert_eq!(vault_names, ["vault01", "vault02"]);
        assert_eq!(vault_list[1].root_token, "hvs.two");
        let keycloak_names: Vec<&str> = keycloak_list.iter().map(|kc| kc.name.as_str()).collect();
        assert_eq!(keycloak_names, ["kc01", "kc02"]);
        assert_eq!(keycloak_list[0].admin, Some(kc01));
        assert_eq!(keycloak_list[1].address, "https://172.31.1.24:8443");
        let surreal_db_names: Vec<&str> =
            surreal_db_list.iter().map(|db| db.name.as_str()).collect();
        assert_eq!(surreal_db_names, ["db01", "db02"]);
        assert_eq!(surreal_db_list[0].admin, db01);
        assert_eq!(surreal_db_list[1].address, "http://172.31.1.26:8000");

        let none = lab.path().join("missing");
        assert_eq!(read_vault_credentials(&none).unwrap(), [] as [VaultInfo; 0]);
        assert_eq!(
            read_keycloak_credentials(&none).unwrap(),
            [] as [KeycloakInfo; 0]
        );
        assert_eq!(
            read_surreal_db_credentials(&none).unwrap(),
            [] as [SurrealDbInfo; 0]
        );
    }

    #[test]
    fn vault_init_007_init_is_sent_again_only_while_vault_is_not_initialised() {
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
    fn vault_init_008_a_vault_initialised_after_a_failed_init_lost_its_init_response() {
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
    fn vault_init_009_a_lost_init_response_says_so_and_how_to_reset_the_node() {
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
    fn vault_init_010_an_unknown_status_after_a_failed_init_says_how_to_reset_the_node() {
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
    async fn vault_init_001_a_new_vault_is_initialised_saved_and_unsealed() {
        let vault = TestVault::start("sherpa-test-vault-init").await;
        let dir = TempDir::new().unwrap();
        let client = vault.client();
        let credentials =
            unseal_with_client(&client, dir.path(), "vault01", TEST_TIMEOUT, &progress())
                .await
                .unwrap();
        assert_eq!(
            load_credentials(dir.path(), "vault01").unwrap(),
            Some(credentials)
        );
        let status = client.seal_status().await.unwrap();
        assert!(status.initialized && !status.sealed, "{status:?}");
        assert!(client.is_active().await.unwrap());
        vault.remove().await;
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon and the hashicorp/vault:1.21 image"]
    async fn vault_init_002_an_initialised_vault_is_not_initialised_again() {
        let vault = TestVault::start("sherpa-test-vault-again").await;
        let dir = TempDir::new().unwrap();
        let client = vault.client();
        let first = unseal_with_client(&client, dir.path(), "vault01", TEST_TIMEOUT, &progress())
            .await
            .unwrap();
        let again = unseal_with_client(&client, dir.path(), "vault01", TEST_TIMEOUT, &progress())
            .await
            .unwrap();
        assert_eq!(again, first);
        assert_eq!(
            load_credentials(dir.path(), "vault01").unwrap(),
            Some(first)
        );
        vault.remove().await;
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon and the hashicorp/vault:1.21 image"]
    async fn vault_init_003_a_vault_initialised_without_saved_credentials_is_refused() {
        let vault = TestVault::start("sherpa-test-vault-outside").await;
        let client = vault.client();
        let initialised = TempDir::new().unwrap();
        unseal_with_client(
            &client,
            initialised.path(),
            "vault01",
            TEST_TIMEOUT,
            &progress(),
        )
        .await
        .unwrap();

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
        assert_eq!(load_credentials(elsewhere.path(), "vault01").unwrap(), None);
        vault.remove().await;
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon and the hashicorp/vault:1.21 image"]
    async fn vault_init_004_a_timed_out_init_that_completes_in_vault_is_a_lost_init_response() {
        let dir = TempDir::new().unwrap();
        // Saved for the node's earlier Vault, as before a redeploy: they must not be used.
        save_credentials(dir.path(), "vault01", &credentials("hvs.stale")).unwrap();

        // Raft init takes seconds, so the request times out and Vault finishes it anyway.
        let vault = TestVault::start("sherpa-test-vault-lost-init").await;
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
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon and the hashicorp/vault:1.21 image"]
    async fn vault_init_005_a_node_restarted_with_empty_storage_gets_new_saved_credentials() {
        let dir = TempDir::new().unwrap();
        save_credentials(dir.path(), "vault01", &credentials("hvs.stale")).unwrap();

        // The reset the lost init response error asks for: the node again with empty storage.
        let vault = TestVault::start("sherpa-test-vault-reset").await;
        let client = vault.client();
        let fresh = unseal_with_client(&client, dir.path(), "vault01", TEST_TIMEOUT, &progress())
            .await
            .unwrap();
        assert_ne!(fresh.root_token, "hvs.stale");
        assert_eq!(
            load_credentials(dir.path(), "vault01").unwrap(),
            Some(fresh)
        );
        let status = client.seal_status().await.unwrap();
        assert!(status.initialized && !status.sealed, "{status:?}");
        vault.remove().await;
    }

    #[tokio::test]
    async fn vault_init_019_a_sealed_vault_without_a_readable_saved_key_is_kept_sealed_and_sent_nothing()
     {
        let dir = TempDir::new().unwrap();
        // Nothing listens there: an unseal sent would come back `NotApplied`.
        let client = absent_vault();
        let outcome = unseal_with_saved_key(&client, dir.path(), "vault01", SEALED).await;
        assert!(
            matches!(outcome, SealOutcome::Sealed(VaultSealed::NoSavedKey)),
            "{outcome:?}"
        );

        // A damaged file, whose TOML error would quote the line holding the key.
        fs::write(
            credentials_path(dir.path(), "vault01"),
            "unseal_key = \"s3cr3t-key\"\nroot_token = hvs.s3cr3t\n",
        )
        .unwrap();
        let outcome = unseal_with_saved_key(&client, dir.path(), "vault01", SEALED).await;
        let SealOutcome::Sealed(VaultSealed::KeyFileUnreadable { cause }) = outcome else {
            panic!("{outcome:?}");
        };
        assert!(cause.contains("not a Vault credentials file"), "{cause}");
        assert!(!cause.contains("s3cr3t"), "{cause}");

        // A file that cannot be read at all.
        fs::create_dir(credentials_path(dir.path(), "vault02")).unwrap();
        let outcome = unseal_with_saved_key(&client, dir.path(), "vault02", SEALED).await;
        assert!(
            matches!(
                outcome,
                SealOutcome::Sealed(VaultSealed::KeyFileUnreadable { .. })
            ),
            "{outcome:?}"
        );
    }

    #[test]
    fn vault_init_023_a_sealed_vault_is_logged_once_at_warn_and_at_info_when_unsealed() {
        let seals = VaultSeals::default();
        let dir = TempDir::new().unwrap();
        fs::write(
            credentials_path(dir.path(), "vault01"),
            "unseal_key = \"s3cr3t-key\"\nroot_token = hvs.s3cr3t\n",
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ((), log) = capture_log(|| {
            for _ in 0..3 {
                let outcome = runtime.block_on(unseal_with_saved_key(
                    &absent_vault(),
                    dir.path(),
                    "vault01",
                    SEALED,
                ));
                record_seal_outcome(&seals, "lab1", "vault01", &outcome);
            }
            // A new reason is logged again, the same one is not.
            for _ in 0..2 {
                record_seal_outcome(
                    &seals,
                    "lab1",
                    "vault01",
                    &SealOutcome::Sealed(VaultSealed::KeyRefused),
                );
            }
            record_seal_outcome(&seals, "lab1", "vault01", &SealOutcome::Unsealed);
            record_seal_outcome(&seals, "lab1", "vault01", &SealOutcome::Unsealed);
        });
        let warns: Vec<&str> = log.lines().filter(|line| line.contains(" WARN ")).collect();
        assert_eq!(warns.len(), 2, "{log}");
        assert!(
            warns[0].contains("Vault sealed: key file unreadable"),
            "{log}"
        );
        assert!(warns[1].contains("Vault sealed: key refused"), "{log}");
        let infos: Vec<&str> = log
            .lines()
            .filter(|line| line.contains(" INFO ") && line.contains("no longer sealed"))
            .collect();
        assert_eq!(infos.len(), 1, "{log}");
        assert!(!log.contains("s3cr3t"), "{log}");
    }

    #[test]
    fn vault_init_024_a_vault_found_unsealed_or_unsealed_now_has_its_sealed_state_cleared() {
        let seals = VaultSeals::default();
        for cleared_by in [SealOutcome::Unsealed, SealOutcome::UnsealedNow] {
            record_seal_outcome(
                &seals,
                "lab1",
                "vault01",
                &SealOutcome::Sealed(VaultSealed::KeyRefused),
            );
            // Not decided: kept.
            record_seal_outcome(
                &seals,
                "lab1",
                "vault01",
                &SealOutcome::NotApplied(anyhow!("connection refused")),
            );
            assert_eq!(
                seals.of_lab("lab1").get("vault01"),
                Some(&VaultSealed::KeyRefused)
            );
            record_seal_outcome(&seals, "lab1", "vault01", &cleared_by);
            assert!(seals.of_lab("lab1").is_empty(), "{cleared_by:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon and the hashicorp/vault:1.21 image"]
    async fn vault_init_017_a_started_vault_sherpa_initialised_is_unsealed_and_active_as_after_resume()
     {
        let (vault, dir) = sealed_test_vault("sherpa-test-vault-resume").await;
        let client = vault.client();
        let outcome = unseal_started_with_client(&client, dir.path(), "vault01", TEST_TIMEOUT)
            .await
            .unwrap();
        assert!(matches!(outcome, SealOutcome::UnsealedNow), "{outcome:?}");
        let status = client.seal_status().await.unwrap();
        assert!(status.initialized && !status.sealed, "{status:?}");
        assert!(client.is_active().await.unwrap());
        vault.remove().await;
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon and the hashicorp/vault:1.21 image"]
    async fn vault_init_020_a_vault_that_refuses_the_saved_key_is_kept_sealed_as_key_refused() {
        let (vault, dir) = sealed_test_vault("sherpa-test-vault-refused").await;
        let client = vault.client();
        let mut saved = load_credentials(dir.path(), "vault01").unwrap().unwrap();
        // Another Vault's key: the same length, so Vault tries it.
        saved.unseal_key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_owned();
        save_credentials(dir.path(), "vault01", &saved).unwrap();

        let status = client.seal_status().await.unwrap();
        let outcome = unseal_with_saved_key(&client, dir.path(), "vault01", status).await;
        assert!(
            matches!(outcome, SealOutcome::Sealed(VaultSealed::KeyRefused)),
            "{outcome:?}"
        );
        assert!(client.seal_status().await.unwrap().sealed);
        vault.remove().await;
    }
}
