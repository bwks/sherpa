use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rand::Rng;
use rand::RngCore;
use tracing::instrument;

use shared::konst::{SHERPA_LABS_PATH, TALOS_SUPPORTED_MINOR_VERSION};
use shared::util::base64_encode_bytes;
use template::{TalosClientCredentials, TalosClusterSecrets};

use crate::tls::generator::{
    CaSpec, ClientSubject, KeyAlgorithm, PemPair, generate_ca, generate_key_pair,
    issue_client_certificate, write_file_with_permissions,
};

/// Validity of cluster CAs: long enough to outlive any lab.
const CA_VALIDITY_DAYS: u32 = 3650;
/// Validity of the admin client certificates handed to the user.
const CLIENT_VALIDITY_DAYS: u32 = 365;
/// Directory under a lab directory holding one subdirectory per cluster.
pub const CLUSTERS_DIR: &str = "clusters";
/// File holding a cluster's secrets, readable only by the server.
const SECRETS_FILE: &str = "secrets.toml";
/// Credential files handed to the user.
pub const TALOSCONFIG_FILE: &str = "talosconfig";
pub const KUBECONFIG_FILE: &str = "kubeconfig";

/// Talos OS (API) CA. Talos clients present certificates whose organization is their role.
pub const OS_CA: CaSpec = CaSpec {
    organization: Some("talos"),
    key_algorithm: KeyAlgorithm::Ed25519,
};
const ETCD_CA: CaSpec = CaSpec {
    organization: Some("etcd"),
    key_algorithm: KeyAlgorithm::EcdsaP256,
};
/// Kubernetes CA. Kubernetes reads a client certificate's organization as its group.
pub const KUBERNETES_CA: CaSpec = CaSpec {
    organization: Some("kubernetes"),
    key_algorithm: KeyAlgorithm::EcdsaP256,
};
const AGGREGATOR_CA: CaSpec = CaSpec {
    organization: None,
    key_algorithm: KeyAlgorithm::EcdsaP256,
};

/// Talos role for full API access.
const TALOS_ADMIN_ROLE: &str = "os:admin";
/// Kubernetes group with cluster-admin rights.
const KUBERNETES_ADMIN_GROUP: &str = "system:masters";
const KUBERNETES_ADMIN_USER: &str = "sherpa-admin";

/// Generate a new set of cluster secrets.
#[instrument(level = "debug")]
pub fn generate_cluster_secrets() -> Result<TalosClusterSecrets> {
    let os = generate_ca(&OS_CA, CA_VALIDITY_DAYS).context("Failed to generate Talos OS CA")?;
    let etcd = generate_ca(&ETCD_CA, CA_VALIDITY_DAYS).context("Failed to generate etcd CA")?;
    let kubernetes = generate_ca(&KUBERNETES_CA, CA_VALIDITY_DAYS)
        .context("Failed to generate Kubernetes CA")?;
    let aggregator = generate_ca(&AGGREGATOR_CA, CA_VALIDITY_DAYS)
        .context("Failed to generate aggregator CA")?;
    let service_account_key = generate_key_pair(KeyAlgorithm::EcdsaP256)
        .context("Failed to generate service account key")?
        .serialize_pem();
    let mut secretbox = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut secretbox);
    Ok(TalosClusterSecrets {
        os_ca_cert: os.cert,
        os_ca_key: os.key,
        etcd_ca_cert: etcd.cert,
        etcd_ca_key: etcd.key,
        kubernetes_ca_cert: kubernetes.cert,
        kubernetes_ca_key: kubernetes.key,
        aggregator_ca_cert: aggregator.cert,
        aggregator_ca_key: aggregator.key,
        service_account_key,
        bootstrap_token: random_token(),
        trustd_token: random_token(),
        secretbox_encryption_secret: base64_encode_bytes(&secretbox),
    })
}

/// A Kubernetes-style token: `[a-z0-9]{6}.[a-z0-9]{16}`.
fn random_token() -> String {
    let mut rng = rand::thread_rng();
    let id = random_lowercase_alphanumeric(&mut rng, 6);
    let secret = random_lowercase_alphanumeric(&mut rng, 16);
    format!("{id}.{secret}")
}

fn random_lowercase_alphanumeric(rng: &mut impl Rng, length: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut output = String::with_capacity(length);
    for _ in 0..length {
        output.push(char::from(ALPHABET[rng.gen_range(0..ALPHABET.len())]));
    }
    output
}

/// Issue the admin client certificate for the Talos API.
pub fn talos_admin_credentials(secrets: &TalosClusterSecrets) -> Result<PemPair> {
    issue_client_certificate(
        &OS_CA,
        &secrets.os_ca_key,
        ClientSubject {
            organization: TALOS_ADMIN_ROLE,
            common_name: None,
        },
        KeyAlgorithm::EcdsaP256,
        CLIENT_VALIDITY_DAYS,
    )
    .context("Failed to issue Talos admin certificate")
}

/// Issue the cluster-admin client certificate for the Kubernetes API.
pub fn kubernetes_admin_credentials(secrets: &TalosClusterSecrets) -> Result<PemPair> {
    issue_client_certificate(
        &KUBERNETES_CA,
        &secrets.kubernetes_ca_key,
        ClientSubject {
            organization: KUBERNETES_ADMIN_GROUP,
            common_name: Some(KUBERNETES_ADMIN_USER),
        },
        KeyAlgorithm::EcdsaP256,
        CLIENT_VALIDITY_DAYS,
    )
    .context("Failed to issue Kubernetes admin certificate")
}

/// Borrow credentials in the form the config renderers take.
pub fn client_credentials<'a>(ca_cert: &'a str, pair: &'a PemPair) -> TalosClientCredentials<'a> {
    TalosClientCredentials {
        ca_cert,
        client_cert: &pair.cert,
        client_key: &pair.key,
    }
}

/// Refuse a Talos image version Sherpa cannot generate cluster configs for.
///
/// Talos changes its config schema between minor versions and decodes it strictly, so the
/// generated documents only fit one minor version.
pub fn ensure_supported_version(node_name: &str, version: &str) -> Result<()> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let minor = version.splitn(3, '.').take(2).collect::<Vec<_>>().join(".");
    if minor != TALOS_SUPPORTED_MINOR_VERSION {
        bail!(
            "Node '{node_name}' uses Talos Linux {version}, but Sherpa builds clusters with Talos {TALOS_SUPPORTED_MINOR_VERSION}.x only. Use a {TALOS_SUPPORTED_MINOR_VERSION}.x image, or remove the node from its cluster and provide your own config with ztp_config"
        );
    }
    Ok(())
}

/// Directory holding one cluster's secrets and credentials.
pub fn cluster_dir(lab_id: &str, cluster_name: &str) -> PathBuf {
    Path::new(SHERPA_LABS_PATH)
        .join(lab_id)
        .join(CLUSTERS_DIR)
        .join(cluster_name)
}

/// Save a cluster's secrets so later operations (redeploy) can build matching configs.
#[instrument(level = "debug", skip(secrets), fields(%cluster_name))]
pub fn save_secrets(dir: &Path, cluster_name: &str, secrets: &TalosClusterSecrets) -> Result<()> {
    let content = toml::to_string(secrets).context("Failed to serialize cluster secrets")?;
    write_file_with_permissions(&dir.join(SECRETS_FILE), content.as_bytes(), 0o600)
        .with_context(|| format!("Failed to save secrets for cluster '{cluster_name}'"))
}

/// Load a cluster's saved secrets.
#[instrument(level = "debug", fields(%cluster_name))]
pub fn load_secrets(dir: &Path, cluster_name: &str) -> Result<TalosClusterSecrets> {
    let path = dir.join(SECRETS_FILE);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read secrets for cluster '{cluster_name}'"))?;
    toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn secrets_have_the_key_types_and_formats_talos_accepts() {
        let secrets = generate_cluster_secrets().unwrap();
        for key in [
            &secrets.os_ca_key,
            &secrets.etcd_ca_key,
            &secrets.kubernetes_ca_key,
            &secrets.aggregator_ca_key,
            &secrets.service_account_key,
        ] {
            assert!(key.starts_with("-----BEGIN PRIVATE KEY-----"), "{key}");
        }
        for token in [&secrets.bootstrap_token, &secrets.trustd_token] {
            let (id, secret) = token.split_once('.').unwrap();
            assert_eq!((id.len(), secret.len()), (6, 16), "{token}");
            assert!(
                token
                    .chars()
                    .all(|c| c == '.' || c.is_ascii_lowercase() || c.is_ascii_digit())
            );
        }
        // 32 random bytes in base64.
        assert_eq!(secrets.secretbox_encryption_secret.len(), 44);
        assert!(secrets.secretbox_encryption_secret.ends_with('='));
        assert_ne!(secrets.bootstrap_token, secrets.trustd_token);
    }

    #[test]
    fn admin_credentials_carry_the_admin_role_and_group() {
        let secrets = generate_cluster_secrets().unwrap();
        let talos = talos_admin_credentials(&secrets).unwrap();
        let kubernetes = kubernetes_admin_credentials(&secrets).unwrap();
        for (pair, ca, subject) in [
            (&talos, &secrets.os_ca_cert, "O=os:admin"),
            (
                &kubernetes,
                &secrets.kubernetes_ca_cert,
                "O=system:masters, CN=sherpa-admin",
            ),
        ] {
            let (_, client_pem) = x509_parser::pem::parse_x509_pem(pair.cert.as_bytes()).unwrap();
            let client = client_pem.parse_x509().unwrap();
            let (_, ca_pem) = x509_parser::pem::parse_x509_pem(ca.as_bytes()).unwrap();
            let ca = ca_pem.parse_x509().unwrap();
            assert_eq!(client.subject().to_string(), subject);
            client.verify_signature(Some(ca.public_key())).unwrap();
        }
    }

    #[test]
    fn only_the_pinned_talos_minor_version_is_supported() {
        for version in ["1.14.0", "1.14.4", "v1.14.4"] {
            ensure_supported_version("cp1", version).unwrap();
        }
        for version in ["1.13.11", "1.15.0", "1.1", "latest"] {
            let error = ensure_supported_version("cp1", version)
                .unwrap_err()
                .to_string();
            assert!(error.contains("1.14.x only"), "{version}: {error}");
        }
    }

    #[test]
    fn saved_secrets_round_trip_and_are_private() {
        let root = tempdir().unwrap();
        let dir = root.path().join("k8s");
        let secrets = generate_cluster_secrets().unwrap();
        save_secrets(&dir, "k8s", &secrets).unwrap();
        assert_eq!(load_secrets(&dir, "k8s").unwrap(), secrets);
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &fs::metadata(dir.join(SECRETS_FILE)).unwrap().permissions(),
        );
        assert_eq!(mode & 0o777, 0o600);
    }
}
