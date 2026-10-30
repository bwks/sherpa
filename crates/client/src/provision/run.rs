//! Run a lab's provisioners on its nodes, several nodes at once, and report the result per
//! node.
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures::stream::{self, StreamExt};
use jato::{PromptCache, SshConfig};

use shared::data::{ProvisionResult, ProvisionStatus};
use shared::konst::{PROVISION_CONCURRENCY, READINESS_TIMEOUT, SHERPA_SSH_CONFIG_FILE};
use shared::util::render_provision_table;
use topology::validate::validate_provisioners;
use topology::{Manifest, Node, Provisioner};

use super::output::NodeOutput;
use super::ssh::{provision, wait_for_node};
use super::transport::SshTarget;
use crate::ssh_client::HostKeyRefresh;

/// What every node of one run shares.
struct LabRun<'a> {
    manifest_dir: &'a Path,
    lab_id: &'a str,
    ready_nodes: Option<&'a [String]>,
    ssh_config: Option<&'a SshConfig>,
    prompts: &'a PromptCache,
    refresh: &'a HostKeyRefresh,
    timeout: Duration,
    /// The longest node name, so every node's output lines up.
    name_width: usize,
}

/// Run the ssh provisioners of the lab's nodes, or of `only_node`, from the lab folder
/// `manifest_dir`, up to the manifest's `provision_concurrency` nodes at once. Each node
/// waits until it is ready, then is provisioned, without waiting for the others. With
/// `ready_nodes`, nodes missing from it are skipped as not ready.
/// `results` holds nodes already provisioned another way, such as container nodes, whose
/// provisioners the server applies. Prints a table of all results and fails if any node was not
/// provisioned. Returns how many nodes were provisioned.
pub async fn provision_lab(
    manifest: &Manifest,
    manifest_dir: &Path,
    lab_id: &str,
    ready_nodes: Option<&[String]>,
    only_node: Option<&str>,
    mut results: Vec<ProvisionResult>,
    refresh: &HostKeyRefresh,
) -> Result<usize> {
    let nodes = nodes_to_provision(manifest, only_node)?;
    if nodes.is_empty() && results.is_empty() {
        return Ok(0);
    }
    validate_provisioners(manifest)?;

    let config_path = manifest_dir.join(SHERPA_SSH_CONFIG_FILE);
    let ssh_config = if nodes.is_empty() {
        None
    } else {
        if !config_path.is_file() {
            bail!(
                "{} not found: run `sherpa up` or `sherpa download` in the lab folder first",
                config_path.display()
            );
        }
        Some(SshConfig::load_file(&config_path).context("Failed to read the lab SSH config")?)
    };
    let prompts = PromptCache::new();
    let lab = LabRun {
        manifest_dir,
        lab_id,
        ready_nodes,
        ssh_config: ssh_config.as_ref(),
        prompts: &prompts,
        refresh,
        timeout: Duration::from_secs(manifest.ready_timeout.unwrap_or(READINESS_TIMEOUT)),
        name_width: name_width(&nodes),
    };
    let concurrency = manifest
        .provision_concurrency
        .unwrap_or(PROVISION_CONCURRENCY);

    let mut runs = Vec::with_capacity(nodes.len());
    for node in &nodes {
        runs.push(provision_one(&lab, node));
    }
    // Unordered, so a node that finishes frees its place for the next at once, rather than
    // after every node started before it.
    let finished: Vec<ProvisionResult> = stream::iter(runs)
        .buffer_unordered(usize::from(concurrency))
        .collect()
        .await;
    results.extend(in_manifest_order(&nodes, finished));

    println!("{}", render_provision_table(&results, lab_id));
    let mut not_provisioned = 0;
    for result in &results {
        if result.status != ProvisionStatus::Provisioned {
            not_provisioned += 1;
        }
    }
    if not_provisioned > 0 {
        bail!(
            "{not_provisioned} of {} nodes were not provisioned; fix the problem, then run \
             `sherpa provision`",
            results.len()
        );
    }
    Ok(results.len())
}

/// Provision one node, unless it was not ready when the lab came up, and report the result.
async fn provision_one(lab: &LabRun<'_>, node: &Node) -> ProvisionResult {
    if let Some(reason) = skip_reason(&node.name, lab.ready_nodes) {
        return ProvisionResult {
            node: node.name.clone(),
            status: ProvisionStatus::Skipped,
            detail: reason,
        };
    }
    let mut output = NodeOutput::new(&node.name, lab.name_width);
    let (status, detail) = match provision_target(lab, node, &mut output).await {
        Ok(()) => (ProvisionStatus::Provisioned, String::new()),
        Err(error) => (ProvisionStatus::Failed, format!("{error:#}")),
    };
    if let Err(error) = output.status(&status.to_string()) {
        tracing::debug!(node = %node.name, error = %format!("{error:#}"), "Cannot show the node's result");
    }
    ProvisionResult {
        node: node.name.clone(),
        status,
        detail,
    }
}

/// Find the node in the lab SSH config, then provision it.
async fn provision_target(lab: &LabRun<'_>, node: &Node, output: &mut NodeOutput) -> Result<()> {
    let ssh_config = lab
        .ssh_config
        .context("The lab SSH config was not loaded")?;
    let target = SshTarget::new(ssh_config, &node.name, lab.lab_id, lab.prompts, lab.refresh)?;
    output.status(&format!("provisioning {}", target.host()))?;
    provision_node(&target, node, lab.manifest_dir, lab.timeout, output).await
}

async fn provision_node(
    target: &SshTarget,
    node: &Node,
    manifest_dir: &Path,
    timeout: Duration,
    output: &mut NodeOutput,
) -> Result<()> {
    let session = wait_for_node(target, timeout).await?;
    let mut outcome = Ok(());
    for provisioner in node.provisioners.as_deref().unwrap_or_default() {
        match provisioner {
            Provisioner::Ssh(ssh) => {
                outcome = provision(&session, ssh, manifest_dir, output).await;
                if outcome.is_err() {
                    break;
                }
            }
            Provisioner::Container(_) => {
                tracing::debug!(node = %node.name, "Container provisioner is applied by the server");
            }
        }
    }
    session.close().await;
    outcome
}

/// Whether a node has an ssh provisioner.
fn has_ssh_provisioner(node: &Node) -> bool {
    for provisioner in node.provisioners.as_deref().unwrap_or_default() {
        if matches!(provisioner, Provisioner::Ssh(_)) {
            return true;
        }
    }
    false
}

/// The nodes with ssh provisioners, or just `only_node`, which must be in the manifest.
fn nodes_to_provision<'a>(
    manifest: &'a Manifest,
    only_node: Option<&str>,
) -> Result<Vec<&'a Node>> {
    let mut nodes = Vec::new();
    let mut found = only_node.is_none();
    for node in &manifest.nodes {
        if let Some(only) = only_node {
            if node.name != only {
                continue;
            }
            found = true;
        }
        if has_ssh_provisioner(node) {
            nodes.push(node);
        }
    }
    if !found {
        bail!(
            "Node '{}' is not in the manifest",
            only_node.unwrap_or_default()
        );
    }
    Ok(nodes)
}

/// `finished`, results in the order nodes finished, in the order of `nodes` instead.
fn in_manifest_order(nodes: &[&Node], mut finished: Vec<ProvisionResult>) -> Vec<ProvisionResult> {
    let mut ordered = Vec::with_capacity(finished.len());
    for node in nodes {
        let mut found = None;
        for (index, result) in finished.iter().enumerate() {
            if result.node == node.name {
                found = Some(index);
                break;
            }
        }
        if let Some(index) = found {
            ordered.push(finished.remove(index));
        }
    }
    ordered.append(&mut finished);
    ordered
}

/// The length of the longest node name.
fn name_width(nodes: &[&Node]) -> usize {
    let mut width = 0;
    for node in nodes {
        width = width.max(node.name.len());
    }
    width
}

/// Why a node is skipped: it was not ready when the lab came up.
fn skip_reason(node: &str, ready_nodes: Option<&[String]>) -> Option<String> {
    let ready = ready_nodes?;
    if ready.contains(&node.to_string()) {
        return None;
    }
    Some("node was not ready when the lab came up".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::ClientConfig;

    fn manifest() -> Manifest {
        toml::from_str(
            r#"
name = "lab"

[[nodes]]
name = "dev01"
model = "ubuntu_linux"

[[nodes.provisioners]]
type = "ssh"
scripts = ["a.sh"]

[[nodes]]
name = "dev02"
model = "ubuntu_linux"

[[nodes]]
name = "dev03"
model = "free_bsd"

[[nodes.provisioners]]
type = "ssh"
scripts = ["a.sh"]

[[nodes]]
name = "db01"
model = "mongo_db"

[[nodes.provisioners]]
type = "container"

[[nodes.provisioners.files]]
src = "password"
name = "password"
"#,
        )
        .unwrap()
    }

    fn names(nodes: Vec<&Node>) -> Vec<String> {
        let mut out = Vec::new();
        for node in nodes {
            out.push(node.name.clone());
        }
        out
    }

    #[test]
    fn only_nodes_with_provisioners_are_provisioned() {
        let manifest = manifest();
        assert_eq!(
            names(nodes_to_provision(&manifest, None).unwrap()),
            vec!["dev01", "dev03"]
        );
        assert_eq!(
            names(nodes_to_provision(&manifest, Some("dev03")).unwrap()),
            vec!["dev03"]
        );
        assert!(
            nodes_to_provision(&manifest, Some("dev02"))
                .unwrap()
                .is_empty()
        );
        // The server applies container provisioners.
        assert!(
            nodes_to_provision(&manifest, Some("db01"))
                .unwrap()
                .is_empty()
        );
    }

    fn refresh() -> HostKeyRefresh {
        HostKeyRefresh::new(
            "abcd1234",
            "wss://sherpa.example:3030/ws",
            &ClientConfig::default(),
            Path::new("/nonexistent"),
        )
    }

    fn result(node: &str, status: ProvisionStatus) -> ProvisionResult {
        ProvisionResult {
            node: node.to_string(),
            status,
            detail: String::new(),
        }
    }

    #[tokio::test]
    async fn earlier_results_are_reported_without_ssh_nodes() {
        let manifest = manifest();
        let provisioned = provision_lab(
            &manifest,
            Path::new("/nonexistent"),
            "abcd1234",
            None,
            Some("db01"),
            vec![result("db01", ProvisionStatus::Provisioned)],
            &refresh(),
        )
        .await
        .unwrap();
        assert_eq!(provisioned, 1);

        let error = provision_lab(
            &manifest,
            Path::new("/nonexistent"),
            "abcd1234",
            None,
            Some("db01"),
            vec![result("db01", ProvisionStatus::Failed)],
            &refresh(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("1 of 1"), "{error}");
    }

    #[test]
    fn results_are_reported_in_manifest_order() {
        let manifest = manifest();
        let nodes = nodes_to_provision(&manifest, None).unwrap();
        let finished = vec![
            result("dev03", ProvisionStatus::Provisioned),
            result("dev01", ProvisionStatus::Failed),
        ];
        let ordered = in_manifest_order(&nodes, finished);
        assert_eq!(ordered[0].node, "dev01");
        assert_eq!(ordered[0].status, ProvisionStatus::Failed);
        assert_eq!(ordered[1].node, "dev03");
        assert_eq!(ordered.len(), 2);
    }

    #[test]
    fn output_is_padded_to_the_longest_node_name() {
        let manifest = manifest();
        let nodes = nodes_to_provision(&manifest, None).unwrap();
        assert_eq!(name_width(&nodes), 5);
        assert_eq!(name_width(&[]), 0);
    }

    #[test]
    fn an_unknown_node_is_an_error() {
        let error = nodes_to_provision(&manifest(), Some("dev09")).unwrap_err();
        assert!(error.to_string().contains("dev09"), "{error}");
    }

    #[test]
    fn nodes_that_were_not_ready_are_skipped() {
        let ready = vec!["dev01".to_string()];
        assert_eq!(skip_reason("dev01", Some(&ready)), None);
        let reason = skip_reason("dev03", Some(&ready)).unwrap();
        assert!(reason.contains("not ready"), "{reason}");
        assert_eq!(skip_reason("dev03", None), None);
    }

    #[tokio::test]
    async fn a_lab_without_provisioners_needs_no_ssh_config() {
        let manifest: Manifest =
            toml::from_str("name = \"lab\"\n[[nodes]]\nname = \"r1\"\nmodel = \"cisco_iosv\"\n")
                .unwrap();
        let provisioned = provision_lab(
            &manifest,
            Path::new("/nonexistent"),
            "abcd1234",
            None,
            None,
            vec![],
            &refresh(),
        )
        .await
        .unwrap();
        assert_eq!(provisioned, 0);
    }
}
