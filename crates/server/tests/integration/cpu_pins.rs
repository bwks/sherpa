//! CPU pins across labs, against the real libvirt: a host CPU pinned to one node's vCPUs is
//! refused to every other node, from the moment the operation is admitted, with the pin
//! refusal (-32006). The tests boot `alpine_linux` VMs and pin host CPUs 2 to 5.

use std::path::Path;
use std::process;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use libvirt::Qemu;
use serde_json::{Value, json};
use shared::data::NodeState;
use shared::error::RpcErrorCode;
use tokio::time;

use crate::helpers::test_server::TestServer;
use crate::helpers::ws_client::TestWsClient;

/// How long one `alpine_linux` lab may take to come up or be redeployed.
const BUILD_TIMEOUT: Duration = Duration::from_secs(300);

/// A lab of one `alpine_linux` VM, `n1`, with its two vCPUs pinned to `cpus`.
fn pinned_lab(lab_id: &str, cpus: [u16; 2]) -> Value {
    json!({
        "name": lab_id,
        "nodes": [
            {
                "name": "n1",
                "model": "alpine_linux",
                "cpu": { "count": 2, "pinning": cpus },
            }
        ]
    })
}

/// An eight-character lab ID unique to this test run.
fn lab_id(prefix: &str) -> String {
    format!("{prefix}{:06}", process::id() % 1_000_000)
}

/// A test server with the images on disk scanned in, and a client logged in as a lab admin.
async fn server() -> Result<(TestServer, TestWsClient, String)> {
    ensure!(
        Path::new("/dev/kvm").exists(),
        "/dev/kvm is required for the CPU pin tests"
    );
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    let (_, scanned) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;
    ensure!(
        scanned.get("error").is_none(),
        "image.scan failed: {scanned}"
    );
    Ok((server, ws, token))
}

/// Start lab `lab_id` from `manifest`, and return the RPC response.
async fn up(ws: &mut TestWsClient, token: &str, lab_id: &str, manifest: &Value) -> Result<Value> {
    let (_, response) = ws
        .rpc_call_streaming(
            "up",
            json!({ "token": token, "lab_id": lab_id, "manifest": manifest }),
            BUILD_TIMEOUT,
        )
        .await?;
    Ok(response)
}

/// Check that `response` is the pin refusal, and return its message.
fn pin_refusal(response: &Value) -> Result<String> {
    let error = response
        .get("error")
        .with_context(|| format!("the request succeeded: {response}"))?;
    ensure!(
        error["code"] == RpcErrorCode::CpuPinsHeld.code(),
        "not the pin refusal: {response}"
    );
    error["message"]
        .as_str()
        .map(str::to_string)
        .with_context(|| format!("the refusal has no message: {response}"))
}

/// Check that nothing of a refused lab was left: no lab record and no domain.
async fn nothing_created(server: &TestServer, lab_id: &str) -> Result<()> {
    ensure!(
        db::find_lab(&server.db, lab_id).await?.is_none(),
        "the refused lab {lab_id} kept its record"
    );
    let conn = Qemu::default().connect()?;
    let domains = libvirt::list_domain_memory(&conn)?;
    ensure!(
        !domains.iter().any(|domain| domain.name.ends_with(lab_id)),
        "the refused lab {lab_id} has a domain"
    );
    Ok(())
}

/// Destroy every lab of `lab_ids` that still exists.
async fn destroy_all(
    ws: &mut TestWsClient,
    server: &TestServer,
    token: &str,
    lab_ids: &[&str],
) -> Result<()> {
    let mut failed = Vec::new();
    for lab_id in lab_ids {
        if db::find_lab(&server.db, lab_id).await?.is_none() {
            continue;
        }
        let (_, response) = ws
            .rpc_call_streaming(
                "destroy",
                json!({ "token": token, "lab_id": lab_id }),
                BUILD_TIMEOUT,
            )
            .await?;
        if response.get("error").is_some() {
            failed.push(format!("{lab_id}: {response}"));
        }
    }
    ensure!(failed.is_empty(), "cleanup failed: {failed:?}");
    Ok(())
}

/// Report a test failure and a cleanup failure together; neither may hide the other.
fn with_cleanup(checks: Result<()>, cleanup: Result<()>) -> Result<()> {
    match (checks, cleanup) {
        (Ok(()), cleanup) => cleanup.context("the test passed but cleanup failed"),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("cleanup failed too: {cleanup:#}")))
        }
    }
}

/// Wait until node `n1` of lab `lab_id` is recorded `state`.
async fn wait_for_n1(server: &TestServer, lab_id: &str, state: NodeState) -> Result<()> {
    let start = Instant::now();
    let lab = db::get_lab(&server.db, lab_id).await?;
    let lab_record = db::get_lab_id(&lab)?;
    loop {
        let node = db::get_node_by_name_and_lab(&server.db, "n1", lab_record.clone()).await?;
        if node.state == state {
            return Ok(());
        }
        if start.elapsed() > BUILD_TIMEOUT {
            bail!(
                "n1 of {lab_id} was not recorded {state}: it is {}",
                node.state
            );
        }
        time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
#[ignore = "requires KVM, libvirt with the alpine_linux image, Docker and root"]
async fn lab_preflight_005_cpus_pinned_by_another_lab_are_refused_naming_the_lab_holding_them()
-> Result<()> {
    let (server, mut ws, token) = server().await?;
    let mut second = TestWsClient::connect(&server).await?;
    let (first_id, second_id) = (lab_id("pc"), lab_id("pd"));
    let checks: Result<()> = async {
        let first_lab = pinned_lab(&first_id, [2, 3]);
        let second_lab = pinned_lab(&second_id, [3, 4]);
        let (first, other) = tokio::join!(
            up(&mut ws, &token, &first_id, &first_lab),
            up(&mut second, &token, &second_id, &second_lab),
        );
        let (first, other) = (first?, other?);
        let (built, refused, refused_id) = match (first.get("error"), other.get("error")) {
            (None, Some(_)) => (&first_id, &other, &second_id),
            (Some(_), None) => (&second_id, &first, &first_id),
            _ => bail!("not exactly one up succeeded:\n{first}\n{other}"),
        };
        let message = pin_refusal(refused)?;
        ensure!(message.contains("Host CPU 3,"), "{message}");
        ensure!(
            message.contains(&format!("of node 'n1' of lab '{built}' ({built})")),
            "an admin sees the holder: {message}"
        );
        nothing_created(&server, refused_id).await
    }
    .await;
    let cleanup = destroy_all(&mut ws, &server, &token, &[&first_id, &second_id]).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires KVM, libvirt with the alpine_linux image, Docker and root"]
async fn lab_redeploy_004_a_redeploy_to_new_pins_holds_its_old_ones_from_a_racing_up_until_it_ends()
-> Result<()> {
    let (server, mut ws, token) = server().await?;
    let mut second_client = TestWsClient::connect(&server).await?;
    let (held_id, racing_id) = (lab_id("pr"), lab_id("ps"));
    let checks: Result<()> = async {
        let response = up(&mut ws, &token, &held_id, &pinned_lab(&held_id, [2, 3])).await?;
        ensure!(response.get("error").is_none(), "up failed: {response}");

        let moved = pinned_lab(&held_id, [4, 5]);
        let redeploy = second_client.rpc_call_streaming(
            "redeploy",
            json!({ "token": token, "lab_id": held_id, "node_name": "n1", "manifest": moved }),
            BUILD_TIMEOUT,
        );
        let racing = async {
            // The node is recorded building once its redeploy is admitted.
            wait_for_n1(&server, &held_id, NodeState::Building).await?;
            let old = up(&mut ws, &token, &racing_id, &pinned_lab(&racing_id, [2, 3])).await?;
            let new = up(&mut ws, &token, &racing_id, &pinned_lab(&racing_id, [5, 6])).await?;
            Ok::<_, anyhow::Error>((old, new))
        };
        let (redeployed, raced) = tokio::join!(redeploy, racing);
        let (_, redeployed) = redeployed?;
        let (old, new) = raced?;
        ensure!(
            redeployed.get("error").is_none(),
            "the redeploy failed: {redeployed}"
        );
        let old = pin_refusal(&old).context("the old pins were taken during the redeploy")?;
        ensure!(old.contains("Host CPUs 2, 3,"), "{old}");
        let new = pin_refusal(&new).context("the new pins were taken during the redeploy")?;
        ensure!(new.contains("Host CPU 5,"), "{new}");
        nothing_created(&server, &racing_id).await?;

        // Once the redeploy is done, the old pins are free and the new ones held.
        let response = up(&mut ws, &token, &racing_id, &pinned_lab(&racing_id, [2, 3])).await?;
        ensure!(
            response.get("error").is_none(),
            "the old pins were kept after the redeploy: {response}"
        );
        Ok(())
    }
    .await;
    let cleanup = destroy_all(&mut ws, &server, &token, &[&held_id, &racing_id]).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires KVM, libvirt with the alpine_linux image, Docker and root"]
async fn lab_redeploy_003_a_refused_redeploy_keeps_the_nodes_cpu_pins() -> Result<()> {
    let (server, mut ws, token) = server().await?;
    let (moving_id, holder_id) = (lab_id("pf"), lab_id("pg"));
    let checks: Result<()> = async {
        for (id, cpus) in [(&moving_id, [2, 3]), (&holder_id, [4, 5])] {
            let response = up(&mut ws, &token, id, &pinned_lab(id, cpus)).await?;
            ensure!(
                response.get("error").is_none(),
                "up of {id} failed: {response}"
            );
        }
        // The redeploy asks for CPUs the other lab holds, and fails before teardown.
        let (_, response) = ws
            .rpc_call_streaming(
                "redeploy",
                json!({
                    "token": token,
                    "lab_id": moving_id,
                    "node_name": "n1",
                    "manifest": pinned_lab(&moving_id, [3, 4]),
                }),
                BUILD_TIMEOUT,
            )
            .await?;
        pin_refusal(&response)?;
        let lab = db::get_lab(&server.db, &moving_id).await?;
        let node = db::get_node_by_name_and_lab(&server.db, "n1", db::get_lab_id(&lab)?).await?;
        ensure!(
            node.state == NodeState::Running,
            "the node was left {}",
            node.state
        );
        let pins: Vec<Vec<u16>> = db::list_cpu_pins(&server.db)
            .await?
            .into_iter()
            .filter(|held| held.lab == node.lab && held.pins.node_name == "n1")
            .map(|held| held.pins.vcpu_cpus)
            .collect();
        ensure!(pins == [vec![2, 3]], "the node's pins changed: {pins:?}");

        // Its old pins are still held.
        let third = lab_id("ph");
        let response = up(&mut ws, &token, &third, &pinned_lab(&third, [1, 2])).await?;
        pin_refusal(&response)?;
        nothing_created(&server, &third).await
    }
    .await;
    let cleanup = destroy_all(&mut ws, &server, &token, &[&moving_id, &holder_id]).await;
    with_cleanup(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires KVM, libvirt with the alpine_linux image, Docker and root"]
async fn lab_destroy_003_a_destroyed_labs_cpu_pins_can_be_pinned_again() -> Result<()> {
    let (server, mut ws, token) = server().await?;
    let (first_id, second_id) = (lab_id("pa"), lab_id("pb"));
    let checks: Result<()> = async {
        let response = up(&mut ws, &token, &first_id, &pinned_lab(&first_id, [2, 3])).await?;
        ensure!(response.get("error").is_none(), "up failed: {response}");
        let second_lab = pinned_lab(&second_id, [2, 3]);
        pin_refusal(&up(&mut ws, &token, &second_id, &second_lab).await?)?;

        destroy_all(&mut ws, &server, &token, &[&first_id]).await?;
        let response = up(&mut ws, &token, &second_id, &second_lab).await?;
        ensure!(
            response.get("error").is_none(),
            "the destroyed lab's pins were kept: {response}"
        );
        Ok(())
    }
    .await;
    let cleanup = destroy_all(&mut ws, &server, &token, &[&first_id, &second_id]).await;
    with_cleanup(checks, cleanup)
}
