//! Vault set-up for the Kubernetes clusters that use a Vault node. Each cluster gets a JWT auth
//! method that accepts its service account tokens, checked against the public half of the
//! cluster's service account signing key, so Vault never calls the cluster. Workloads log in
//! with role `workload` and read the KV secrets under `secret/<cluster>/`.

use anyhow::{Context, Result};
use serde_json::json;
use tracing::instrument;

use shared::konst::{VAULT_KUBERNETES_AUDIENCE, VAULT_KUBERNETES_ROLE};

use super::api::{MountKind, VaultClient};

/// Mount path of the KV version 2 secrets engine clusters read from.
pub const VAULT_KV_MOUNT: &str = "secret";
/// Lifetime of the Vault token a workload gets when it logs in.
const WORKLOAD_TOKEN_TTL: &str = "1h";

/// What Vault needs to accept one cluster's service account tokens.
#[derive(Debug)]
pub struct ClusterAccess<'a> {
    pub cluster_name: &'a str,
    /// The cluster's service account token issuer, its API endpoint URL.
    pub issuer: &'a str,
    /// Public key (PEM) the cluster signs service account tokens with.
    pub service_account_public_key: &'a str,
}

/// Mount path of the JWT auth method that accepts a cluster's service account tokens.
pub fn cluster_auth_mount(cluster_name: &str) -> String {
    format!("kubernetes-{cluster_name}")
}

/// Policy letting a cluster's workloads read the secrets under `secret/<cluster>/`.
fn cluster_policy(cluster_name: &str) -> String {
    format!(
        r#"path "{VAULT_KV_MOUNT}/data/{cluster_name}/*" {{
  capabilities = ["read"]
}}
path "{VAULT_KV_MOUNT}/metadata/{cluster_name}/*" {{
  capabilities = ["read", "list"]
}}
"#
    )
}

/// Let a cluster's workloads log in to Vault and read their secrets: enable the KV secrets
/// engine and the cluster's JWT auth method if they are not enabled yet, then write the auth
/// method's config, the cluster's policy and the `workload` role.
#[instrument(skip(client, root_token, access), fields(cluster = %access.cluster_name, vault = %client.address()))]
pub async fn configure_cluster_access(
    client: &VaultClient,
    root_token: &str,
    access: &ClusterAccess<'_>,
) -> Result<()> {
    let cluster_name = access.cluster_name;
    let mount = cluster_auth_mount(cluster_name);

    let engines = client
        .mount_paths(root_token, MountKind::SecretsEngine)
        .await?;
    if !engines.contains(&format!("{VAULT_KV_MOUNT}/")) {
        client
            .write(
                root_token,
                &format!("sys/mounts/{VAULT_KV_MOUNT}"),
                &json!({ "type": "kv", "options": { "version": "2" } }),
            )
            .await
            .context("Failed to enable the KV secrets engine")?;
    }
    let auth_methods = client
        .mount_paths(root_token, MountKind::AuthMethod)
        .await?;
    if !auth_methods.contains(&format!("{mount}/")) {
        client
            .write(
                root_token,
                &format!("sys/auth/{mount}"),
                &json!({ "type": "jwt" }),
            )
            .await
            .with_context(|| {
                format!("Failed to enable the auth method of cluster '{cluster_name}'")
            })?;
    }
    client
        .write(
            root_token,
            &format!("auth/{mount}/config"),
            &json!({
                "jwt_validation_pubkeys": [access.service_account_public_key],
                "bound_issuer": access.issuer,
            }),
        )
        .await?;
    client
        .write(
            root_token,
            &format!("sys/policies/acl/{mount}"),
            &json!({ "policy": cluster_policy(cluster_name) }),
        )
        .await?;
    client
        .write(
            root_token,
            &format!("auth/{mount}/role/{VAULT_KUBERNETES_ROLE}"),
            &json!({
                "role_type": "jwt",
                "bound_audiences": [VAULT_KUBERNETES_AUDIENCE],
                "user_claim": "sub",
                "bound_claims_type": "glob",
                "bound_claims": { "sub": "system:serviceaccount:*" },
                "token_policies": [mount],
                "token_ttl": WORKLOAD_TOKEN_TTL,
            }),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cluster_has_its_own_auth_mount() {
        assert_eq!(cluster_auth_mount("k8s"), "kubernetes-k8s");
    }

    #[test]
    fn workloads_read_only_their_clusters_secrets() {
        assert_eq!(
            cluster_policy("k8s"),
            r#"path "secret/data/k8s/*" {
  capabilities = ["read"]
}
path "secret/metadata/k8s/*" {
  capabilities = ["read", "list"]
}
"#
        );
    }
}
