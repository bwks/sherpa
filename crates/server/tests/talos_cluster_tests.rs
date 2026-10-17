mod helpers;

use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use helpers::test_server::TestServer;
use helpers::ws_client::TestWsClient;

/// Talos version of the image under `/opt/sherpa/images/talos_linux/`.
const DEFAULT_TALOS_VERSION: &str = "1.14.4";
const LAB_TIMEOUT: Duration = Duration::from_secs(1200);

fn talos_version() -> String {
    std::env::var("SHERPA_TEST_TALOS_VERSION").unwrap_or_else(|_| DEFAULT_TALOS_VERSION.to_owned())
}

fn cluster_manifest(lab_name: &str, version: &str) -> Value {
    json!({
        "name": lab_name,
        "clusters": [{ "name": "k8s" }],
        "nodes": [
            { "name": "cp1", "model": "talos_linux", "version": version, "cluster": "k8s", "cluster_role": "control_plane" },
            { "name": "w1", "model": "talos_linux", "version": version, "cluster": "k8s", "cluster_role": "worker" },
        ],
    })
}

async fn scan_images(ws: &mut TestWsClient, token: &str) -> Result<()> {
    let (_, response) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;
    ensure!(
        response.get("error").is_none(),
        "image.scan failed: {response}"
    );
    Ok(())
}

async fn destroy_lab(
    ws: &mut TestWsClient,
    server: &TestServer,
    token: &str,
    lab_id: &str,
) -> Result<()> {
    if db::find_lab(&server.db, lab_id).await?.is_none() {
        return Ok(());
    }
    let (_, response) = ws
        .rpc_call_streaming(
            "destroy",
            json!({ "token": token, "lab_id": lab_id }),
            Duration::from_secs(300),
        )
        .await?;
    ensure!(
        response.get("error").is_none(),
        "Lab {lab_id} cleanup failed: {response}"
    );
    Ok(())
}

/// Count Ready nodes using only the kubeconfig the user was given.
async fn ready_kubernetes_nodes(kubeconfig: &str) -> Result<usize> {
    let config: Value = serde_json::from_str(kubeconfig).context("kubeconfig is not JSON")?;
    let decode = |value: &Value| -> Result<String> {
        shared::util::base64_decode(value.as_str().context("missing kubeconfig field")?)
    };
    let cluster = &config["clusters"][0]["cluster"];
    let user = &config["users"][0]["user"];
    let mut identity = decode(&user["client-certificate-data"])?;
    identity.push_str(&decode(&user["client-key-data"])?);
    let client = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(
            decode(&cluster["certificate-authority-data"])?.as_bytes(),
        )?])
        .identity(reqwest::Identity::from_pem(identity.as_bytes())?)
        .timeout(Duration::from_secs(20))
        .build()?;
    let server = cluster["server"].as_str().context("missing server")?;
    let nodes: Value = client
        .get(format!("{server}/api/v1/nodes"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let ready = nodes["items"]
        .as_array()
        .context("node list has no items")?
        .iter()
        .filter(|node| {
            node["status"]["conditions"]
                .as_array()
                .is_some_and(|conditions| {
                    conditions.iter().any(|condition| {
                        condition["type"] == "Ready" && condition["status"] == "True"
                    })
                })
        })
        .count();
    Ok(ready)
}

async fn wait_for_ready_nodes(kubeconfig: &str, expected: usize, timeout: Duration) -> Result<()> {
    let started = Instant::now();
    loop {
        match ready_kubernetes_nodes(kubeconfig).await {
            Ok(ready) if ready >= expected => return Ok(()),
            Ok(ready) => eprintln!("{ready}/{expected} Kubernetes nodes ready"),
            Err(error) => eprintln!("Kubernetes API not ready: {error:#}"),
        }
        ensure!(
            started.elapsed() < timeout,
            "Kubernetes nodes did not become Ready"
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

#[tokio::test]
#[ignore = "requires root, KVM, libvirt, internet access and the talos_linux 1.14 image"]
async fn talos_cluster_comes_up_with_working_credentials() -> Result<()> {
    // A missing requirement is a failure, not a pass: this test must actually run.
    ensure!(
        std::path::Path::new("/dev/kvm").exists(),
        "/dev/kvm is required"
    );
    let version = talos_version();
    let image = format!("/opt/sherpa/images/talos_linux/{version}/virtioa.qcow2");
    ensure!(
        std::path::Path::new(&image).exists(),
        "Talos image missing: {image} (sherpa server image pull --model talos_linux --version {version})"
    );

    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    scan_images(&mut ws, &token).await?;

    let lab_id = format!("tk{:06}", std::process::id() % 1_000_000);
    let manifest = cluster_manifest(&lab_id, &version);

    let checks: Result<()> = async {
        let (_, response) = ws
            .rpc_call_streaming("up", json!({ "token": token, "lab_id": lab_id, "manifest": manifest }), LAB_TIMEOUT)
            .await?;
        ensure!(response.get("error").is_none(), "up failed: {response}");
        let result = &response["result"];
        let not_ready: Vec<&Value> = result["errors"]
            .as_array()
            .map(|errors| errors.iter().filter(|e| e["message"].as_str().is_some_and(|m| m.contains("did not become ready"))).collect())
            .unwrap_or_default();
        ensure!(not_ready.is_empty(), "lab was not ready: {not_ready:?}");

        let cluster = &result["clusters"][0];
        ensure!(cluster["name"] == "k8s", "missing cluster credentials: {result}");
        let kubeconfig = cluster["kubeconfig"].as_str().context("missing kubeconfig")?.to_owned();
        let talosconfig: Value = serde_json::from_str(cluster["talosconfig"].as_str().context("missing talosconfig")?)?;
        ensure!(talosconfig["contexts"]["k8s"]["endpoints"][0].is_string(), "talosconfig has no k8s context: {talosconfig}");
        ensure!(ready_kubernetes_nodes(&kubeconfig).await? == 2, "expected 2 Ready nodes");

        // Talos nodes have no SSH, so they are left out of the SSH config.
        ensure!(!result["ssh_config"].as_str().unwrap_or_default().contains("cp1"), "Talos node in SSH config");

        let (_, refused) = ws
            .rpc_call_streaming("redeploy", json!({ "token": token, "lab_id": lab_id, "node_name": "cp1", "manifest": manifest }), LAB_TIMEOUT)
            .await?;
        ensure!(
            refused["error"]["context"].as_str().is_some_and(|c| c.contains("control plane of cluster 'k8s'")),
            "control plane redeploy was not refused: {refused}"
        );
        ensure!(ready_kubernetes_nodes(&kubeconfig).await? == 2, "refused redeploy changed the cluster");

        let (_, redeployed) = ws
            .rpc_call_streaming("redeploy", json!({ "token": token, "lab_id": lab_id, "node_name": "w1", "manifest": manifest }), LAB_TIMEOUT)
            .await?;
        ensure!(redeployed.get("error").is_none(), "worker redeploy failed: {redeployed}");
        wait_for_ready_nodes(&kubeconfig, 2, Duration::from_secs(600)).await?;

        let (_, download) = ws
            .rpc_call_streaming("download", json!({ "token": token, "lab_id": lab_id }), Duration::from_secs(60))
            .await?;
        ensure!(download["result"]["clusters"][0]["kubeconfig"] == kubeconfig, "download did not return the cluster credentials");
        Ok(())
    }
    .await;

    let keep =
        checks.is_err() && std::env::var("SHERPA_TEST_KEEP_FAILED_LABS").as_deref() == Ok("1");
    let cleanup = if keep {
        eprintln!("Keeping failed lab {lab_id} for debugging");
        Ok(())
    } else {
        destroy_lab(&mut ws, &server, &token, &lab_id).await
    };
    match (checks, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(cleanup)) => Err(cleanup.context("Test passed but lab cleanup failed")),
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("Lab cleanup also failed: {cleanup:#}")))
        }
    }
}
