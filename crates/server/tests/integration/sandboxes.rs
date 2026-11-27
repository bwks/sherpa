use anyhow::{Context, Result, bail, ensure};
use bollard::Docker;
use db::DefaultRequest;
use serde_json::{Value, json};
use shared::data::{
    ContainerSettings, KindSettings, ModelKind, NodeConfig, NodeKind, NodeModel, NodeState,
    ZtpSettings,
};
use shared::konst::{SHERPA_LABS_PATH, SHERPA_STORAGE_POOL_PATH};
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{self, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};
use tokio::time;

use crate::helpers::test_server::TestServer;
use crate::helpers::ws_client::TestWsClient;
use crate::lab_lifecycle::{
    cleanup_lab, lab_up, libvirt_leftovers, node_states, require_kvm, with_cleanup,
};

/// The repo of the test image: Alpine with an entrypoint that sleeps whatever arguments follow,
/// as a `sonic_linux` container gets `-c` and its start files as its arguments.
const TEST_IMAGE_REPO: &str = "sherpa-sandbox-test";

/// The version of the test image that starts.
const TEST_IMAGE_VERSION: &str = "1";

/// A volume source that does not exist on the server.
const MISSING_VOLUME_SOURCE: &str = "/nonexistent/sherpa-sandbox-test";

/// The Dockerfile of the test image.
const TEST_IMAGE_DOCKERFILE: &str = r#"FROM alpine:3.22
ENTRYPOINT ["/bin/sh", "-c", "exec sleep 100000"]
"#;

/// The version of the test image that is plain Alpine, with no entrypoint, so the `-c` it is
/// given is taken as its program.
const BROKEN_IMAGE_VERSION: &str = "broken";

/// How long a lab of sandboxes may take to come up, which includes making a Docker store.
const UP_TIMEOUT: Duration = Duration::from_mins(15);

/// How long down, resume, redeploy and destroy may take.
const OPERATION_TIMEOUT: Duration = Duration::from_mins(10);

/// How long a bridge member tap or a node state may take to show.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);

/// The folder of the labs' console sockets.
const CONSOLE_DIR: &str = "/opt/sherpa/run/console";

/// The folder of the status sockets of the VMs that make Docker stores.
const SANDBOX_RUN_DIR: &str = "/opt/sherpa/run/sandbox";

/// Report the checks and the cleanup together, after removing the store status folder when it
/// is empty, as one these tests made as root would refuse sherpad, which makes it again.
fn finish(checks: Result<()>, cleanup: Result<()>) -> Result<()> {
    let _ = fs::remove_dir(SANDBOX_RUN_DIR);
    with_cleanup(checks, cleanup)
}

/// The memory of a node's container, in MiB; its VM has 1024 more.
const CONTAINER_MEMORY_MIB: u16 = 128;

/// Build the test images in the server's Docker, a no-op once they exist.
fn build_test_image() -> Result<()> {
    let mut build = Command::new("docker")
        .args(["build", "--quiet", "--tag"])
        .arg(format!("{TEST_IMAGE_REPO}:{TEST_IMAGE_VERSION}"))
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("Failed to run docker build")?;
    build
        .stdin
        .take()
        .context("docker build has no stdin")?
        .write_all(TEST_IMAGE_DOCKERFILE.as_bytes())
        .context("Failed to send the Dockerfile to docker build")?;
    let status = build.wait().context("Failed to wait for docker build")?;
    ensure!(status.success(), "docker build of the test image failed");
    let tagged = Command::new("docker")
        .args(["tag", "alpine:3.22"])
        .arg(format!("{TEST_IMAGE_REPO}:{BROKEN_IMAGE_VERSION}"))
        .status()
        .context("Failed to run docker tag")?;
    ensure!(
        tagged.success(),
        "docker tag of the broken test image failed"
    );
    Ok(())
}

/// Register a `sonic_linux` container image of `repo` and `version`.
async fn register_image(
    server: &TestServer,
    repo: &str,
    version: &str,
    default: bool,
) -> Result<()> {
    let image = NodeConfig {
        settings: KindSettings::Container(ContainerSettings {
            repo: Some(repo.to_owned()),
            ztp: ZtpSettings::default(),
        }),
        version: version.to_owned(),
        default,
        ..ModelKind::new(NodeModel::SonicLinux, Some(NodeKind::Container))
            .context("sonic_linux has no container kind")?
            .node_config()
    };
    db::upsert_node_image(&server.db, image, DefaultRequest::Keep).await?;
    Ok(())
}

/// A test server with the images the tests use registered, and an admin logged in.
async fn start() -> Result<(TestServer, TestWsClient, String)> {
    require_kvm()?;
    build_test_image()?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_lab_admin(&server).await?;
    register_image(&server, TEST_IMAGE_REPO, TEST_IMAGE_VERSION, true).await?;
    register_image(&server, TEST_IMAGE_REPO, BROKEN_IMAGE_VERSION, false).await?;
    Ok((server, ws, token))
}

/// A `sonic_linux` node on `version` of its image, ready without waiting for SSH.
fn node(name: &str, version: &str) -> Value {
    json!({
        "name": name,
        "model": "sonic_linux",
        "kind": "container",
        "version": version,
        "memory": CONTAINER_MEMORY_MIB,
        "ready_check": { "skip": true },
        "interfaces": { "data_count": 2 },
    })
}

/// A lab of nodes `n1` and `n2` on the image that starts, with `connections` added to it.
fn lab_manifest(lab_id: &str, connections: &Value) -> Value {
    let mut manifest = json!({
        "name": lab_id,
        "nodes": [
            node("n1", TEST_IMAGE_VERSION),
            node("n2", TEST_IMAGE_VERSION),
        ],
    });
    if let (Some(manifest), Some(connections)) = (manifest.as_object_mut(), connections.as_object())
    {
        manifest.extend(connections.clone());
    }
    manifest
}

/// The connections of nodes `n1` and `n2` by a P2p link.
fn p2p_link() -> Value {
    json!({ "links": [{ "src": "n1::eth1", "dst": "n2::eth1" }] })
}

/// The connections of nodes `n1` and `n2` by a private bridge.
fn private_bridge() -> Value {
    json!({ "bridges": [{ "name": "private", "links": ["n1::eth1", "n2::eth1"] }] })
}

/// A lab ID of exactly 8 characters, `prefix` and the test process's number.
fn lab_id(prefix: &str) -> String {
    format!("{prefix}{:06}", process::id() % 1_000_000)
}

/// Run `method` on lab `lab_id` and fail when it is refused.
async fn lab_call(
    ws: &mut TestWsClient,
    token: &str,
    method: &str,
    lab_id: &str,
    extra: Value,
) -> Result<Value> {
    let mut params = json!({ "token": token, "lab_id": lab_id });
    if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
        params.extend(extra.clone());
    }
    let (_statuses, response) = ws
        .rpc_call_streaming(method, params, OPERATION_TIMEOUT)
        .await?;
    ensure!(
        response.get("error").is_none(),
        "{method} of {lab_id} failed: {response}"
    );
    Ok(response)
}

/// Wait until every node of lab `lab_id` is recorded `state`.
async fn wait_for_states(server: &TestServer, lab_id: &str, state: NodeState) -> Result<()> {
    let start = Instant::now();
    loop {
        let states = node_states(server, lab_id).await?;
        if !states.is_empty() && states.values().all(|found| *found == state) {
            return Ok(());
        }
        ensure!(
            start.elapsed() < SETTLE_TIMEOUT,
            "the nodes of {lab_id} were not all {state} within {SETTLE_TIMEOUT:?}: {states:?}"
        );
        time::sleep(Duration::from_millis(250)).await;
    }
}

/// The names of the host interfaces that belong to lab `lab_id`.
async fn lab_interfaces(lab_id: &str) -> Result<Vec<String>> {
    let suffix = format!("-{lab_id}");
    Ok(network::list_interface_names()
        .await?
        .into_iter()
        .filter(|name| name.ends_with(&suffix))
        .collect())
}

/// The bridge interface of the host that interface `name` is a port of, if it is one.
fn master_of(name: &str) -> Option<String> {
    fs::read_link(format!("/sys/class/net/{name}/master"))
        .ok()
        .and_then(|target| {
            target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
}

/// Wait until interface `tap` is a port of bridge `bridge`.
async fn wait_for_master(tap: &str, bridge: &str) -> Result<()> {
    let start = Instant::now();
    loop {
        let master = master_of(tap);
        if master.as_deref() == Some(bridge) {
            return Ok(());
        }
        ensure!(
            start.elapsed() < SETTLE_TIMEOUT,
            "{tap} is a port of {master:?}, not of {bridge}, after {SETTLE_TIMEOUT:?}"
        );
        time::sleep(Duration::from_millis(250)).await;
    }
}

/// The ports of the host bridge `bridge`, by name.
fn bridge_ports(bridge: &str) -> Result<Vec<String>> {
    let mut ports = Vec::new();
    for entry in fs::read_dir(format!("/sys/class/net/{bridge}/brif"))
        .with_context(|| format!("Cannot list the ports of {bridge}"))?
    {
        ports.push(entry?.file_name().to_string_lossy().into_owned());
    }
    ports.sort();
    Ok(ports)
}

/// What identifies the file of storage pool volume `volume`: its inode and creation time, which
/// a file made again does not keep.
fn volume_identity(volume: &str) -> Result<(u64, SystemTime)> {
    let metadata = fs::metadata(format!("{SHERPA_STORAGE_POOL_PATH}/{volume}"))
        .with_context(|| format!("Volume {volume} is missing"))?;
    Ok((metadata.ino(), metadata.created()?))
}

/// The Docker store volume of image `image`, named by the first 16 hex digits of its ID.
async fn store_volume(image: &str) -> Result<String> {
    let docker = Docker::connect_with_local_defaults()?;
    let id = docker
        .inspect_image(image)
        .await
        .with_context(|| format!("Image {image} is not in Docker"))?
        .id
        .with_context(|| format!("Image {image} has no ID"))?;
    let digits = id.strip_prefix("sha256:").unwrap_or(&id);
    Ok(format!("flatcar-docker-{}.qcow2", &digits[..16]))
}

/// The ifindex of interface `name`.
async fn ifindex(name: &str) -> Result<u32> {
    network::find_ifindex(name)
        .await?
        .with_context(|| format!("Interface {name} does not exist"))
}

/// Fail unless the eBPF redirect of a P2p link is attached to both of its taps.
async fn ensure_link_attached(server: &TestServer, lab_id: &str) -> Result<()> {
    let lab = db::get_lab(&server.db, lab_id).await?;
    let links = db::list_links_by_lab(&server.db, lab.id.context("lab has no record ID")?).await?;
    ensure!(!links.is_empty(), "lab {lab_id} has no link");
    for link in links {
        for tap in [&link.tap_a, &link.tap_b] {
            ensure!(
                network::has_p2p_redirect(ifindex(tap).await?).await?,
                "{tap} has no redirect to its peer"
            );
        }
    }
    Ok(())
}

/// What lab `lab_id` left on the host outside libvirt's names: its interfaces, its console
/// folder, its lab folder and its record.
async fn host_leftovers(server: &TestServer, lab_id: &str) -> Result<Vec<String>> {
    let mut leftovers = lab_interfaces(lab_id).await?;
    for folder in [
        format!("{CONSOLE_DIR}/{lab_id}"),
        format!("{SHERPA_LABS_PATH}/{lab_id}"),
    ] {
        if Path::new(&folder).exists() {
            leftovers.push(folder);
        }
    }
    if db::find_lab(&server.db, lab_id).await?.is_some() {
        leftovers.push(format!("the record of lab {lab_id}"));
    }
    Ok(leftovers)
}

#[tokio::test]
#[ignore = "requires KVM, libvirt, Docker, the sandbox base image and root"]
async fn sandbox_067_a_sandbox_on_a_private_bridge_is_on_it_again_after_down_and_resume()
-> Result<()> {
    let (server, mut ws, token) = start().await?;
    let lab = lab_id("nb");
    let manifest = lab_manifest(&lab, &private_bridge());

    let checks: Result<()> = async {
        lab_up(&mut ws, &token, &lab, &manifest, UP_TIMEOUT).await?;
        let record = db::get_lab(&server.db, &lab).await?;
        let bridges =
            db::list_bridges(&server.db, &record.id.context("lab has no record ID")?).await?;
        let [bridge] = bridges.as_slice() else {
            bail!("the lab has {} bridges, not 1", bridges.len());
        };
        let bridge = bridge.bridge_name.clone();
        let taps = bridge_ports(&bridge)?;
        ensure!(taps.len() == 2, "the bridge has ports {taps:?}, not 2");

        lab_call(&mut ws, &token, "down", &lab, json!({})).await?;
        lab_call(&mut ws, &token, "resume", &lab, json!({})).await?;
        wait_for_states(&server, &lab, NodeState::Running).await?;
        for tap in &taps {
            wait_for_master(tap, &bridge).await?;
        }
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab, &checks).await;
    finish(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires KVM, libvirt, Docker, the sandbox base image and root"]
async fn sandbox_025_a_redeploy_gives_the_node_a_new_disk_and_config_drive_and_attaches_its_links()
-> Result<()> {
    let (server, mut ws, token) = start().await?;
    let lab = lab_id("nr");
    let manifest = lab_manifest(&lab, &p2p_link());
    let volumes = [format!("n1-{lab}-hdd.qcow2"), format!("n1-{lab}.iso")];

    let checks: Result<()> = async {
        lab_up(&mut ws, &token, &lab, &manifest, UP_TIMEOUT).await?;
        ensure_link_attached(&server, &lab).await?;
        let before = volumes
            .iter()
            .map(|volume| volume_identity(volume))
            .collect::<Result<Vec<_>>>()?;

        lab_call(
            &mut ws,
            &token,
            "redeploy",
            &lab,
            json!({ "node_name": "n1", "manifest": manifest }),
        )
        .await?;

        wait_for_states(&server, &lab, NodeState::Running).await?;
        for (volume, before) in volumes.iter().zip(before) {
            ensure!(
                volume_identity(volume)? != before,
                "redeploy kept the volume {volume}"
            );
        }
        ensure_link_attached(&server, &lab).await
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab, &checks).await;
    finish(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires KVM, libvirt, Docker, the sandbox base image and root"]
async fn sandbox_026_destroy_leaves_no_domain_volume_tap_or_console_folder_and_keeps_the_docker_store()
-> Result<()> {
    let (server, mut ws, token) = start().await?;
    let lab = lab_id("nd");
    let manifest = lab_manifest(&lab, &p2p_link());
    let store = store_volume(&format!("{TEST_IMAGE_REPO}:{TEST_IMAGE_VERSION}")).await?;

    let checks: Result<()> = async {
        lab_up(&mut ws, &token, &lab, &manifest, UP_TIMEOUT).await?;
        ensure!(
            !libvirt_leftovers(&lab)?.is_empty() && !lab_interfaces(&lab).await?.is_empty(),
            "up made no sandbox resources to destroy"
        );
        ensure!(
            Path::new(&format!("{CONSOLE_DIR}/{lab}")).exists(),
            "up made no console folder"
        );
        let store_before = volume_identity(&store)?;

        lab_call(&mut ws, &token, "destroy", &lab, json!({})).await?;

        let libvirt = libvirt_leftovers(&lab)?;
        ensure!(libvirt.is_empty(), "destroy left {libvirt:?} in libvirt");
        let host = host_leftovers(&server, &lab).await?;
        ensure!(host.is_empty(), "destroy left {host:?} on the host");
        ensure!(
            volume_identity(&store)? == store_before,
            "destroy changed the Docker store {store}"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab, &checks).await;
    finish(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires KVM, libvirt, Docker, the sandbox base image and root"]
async fn sandbox_027_an_up_that_fails_creating_sandboxes_is_rolled_back_with_nothing_of_the_lab_left()
-> Result<()> {
    let (server, mut ws, token) = start().await?;
    let lab = lab_id("nf");
    let mut missing_volume = node("n2", TEST_IMAGE_VERSION);
    missing_volume["container"] = json!({
        "volumes": [{ "src": MISSING_VOLUME_SOURCE, "dst": "/etc/sherpa-test" }],
    });
    let manifest = json!({
        "name": lab,
        "nodes": [node("n1", TEST_IMAGE_VERSION), missing_volume],
    });

    let checks: Result<()> = async {
        let (statuses, response) = ws
            .rpc_call_streaming(
                "up",
                json!({ "token": token, "lab_id": lab, "manifest": manifest }),
                UP_TIMEOUT,
            )
            .await?;
        let said = |text: &str| {
            statuses
                .iter()
                .any(|status| status.to_string().contains(text))
        };
        ensure!(
            response.get("error").is_some()
                && said("Creating 2 sandboxes in parallel")
                && !said("All sandboxes created"),
            "the up did not fail while creating its sandboxes: {response} {statuses:?}"
        );
        let libvirt = libvirt_leftovers(&lab)?;
        ensure!(
            libvirt.is_empty(),
            "the failed up left {libvirt:?} in libvirt"
        );
        let host = host_leftovers(&server, &lab).await?;
        ensure!(host.is_empty(), "the failed up left {host:?} on the host");
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab, &checks).await;
    finish(checks, cleanup)
}

#[tokio::test]
#[ignore = "requires KVM, libvirt, Docker, the sandbox base image and root"]
async fn sandbox_038_a_second_lab_with_the_same_image_boots_on_the_store_the_first_made()
-> Result<()> {
    let (server, mut ws, token) = start().await?;
    let first = lab_id("na");
    let second = lab_id("nc");
    let store = store_volume(&format!("{TEST_IMAGE_REPO}:{TEST_IMAGE_VERSION}")).await?;

    let checks: Result<()> = async {
        lab_up(
            &mut ws,
            &token,
            &first,
            &lab_manifest(&first, &json!({})),
            UP_TIMEOUT,
        )
        .await?;
        let store_before = volume_identity(&store)?;
        let statuses = lab_up(
            &mut ws,
            &token,
            &second,
            &lab_manifest(&second, &json!({})),
            UP_TIMEOUT,
        )
        .await?;

        ensure!(
            volume_identity(&store)? == store_before,
            "the second lab replaced the Docker store {store}"
        );
        let building = statuses
            .iter()
            .filter(|status| status.to_string().contains("Docker store"))
            .count();
        ensure!(
            building == 0,
            "the second up reported a store: {statuses:?}"
        );
        wait_for_states(&server, &second, NodeState::Running).await
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &first, &checks).await;
    let cleanup_second = cleanup_lab(&mut ws, &server, &token, &second, &checks).await;
    finish(checks, cleanup.and(cleanup_second))
}

#[tokio::test]
#[ignore = "requires KVM, libvirt, Docker, the sandbox base image and root"]
async fn sandbox_055_a_sandbox_whose_container_cannot_start_fails_with_the_reason_before_the_ready_timeout()
-> Result<()> {
    let (server, mut ws, token) = start().await?;
    let lab = lab_id("ns");
    let ready_timeout = 600;
    let manifest = json!({
        "name": lab,
        "ready_timeout": ready_timeout,
        "nodes": [node("n1", BROKEN_IMAGE_VERSION)],
    });

    let checks: Result<()> = async {
        // The first up of the image makes its store, which is no part of what is timed.
        let _ = ws
            .rpc_call_streaming(
                "up",
                json!({ "token": token, "lab_id": lab, "manifest": manifest }),
                UP_TIMEOUT,
            )
            .await?;
        lab_call(&mut ws, &token, "destroy", &lab, json!({})).await?;

        let start = Instant::now();
        let (_statuses, response) = ws
            .rpc_call_streaming(
                "up",
                json!({ "token": token, "lab_id": lab, "manifest": manifest }),
                UP_TIMEOUT,
            )
            .await?;
        let elapsed = start.elapsed();
        ensure!(
            response.to_string().contains("executable file not found"),
            "up did not report why the container cannot start: {response}"
        );
        ensure!(
            elapsed < Duration::from_secs(ready_timeout) / 2,
            "up took {elapsed:?} to fail the node"
        );
        let states = node_states(&server, &lab).await?;
        ensure!(
            states.get("n1") == Some(&NodeState::Failed),
            "the node is {states:?}, not failed"
        );
        Ok(())
    }
    .await;
    let cleanup = cleanup_lab(&mut ws, &server, &token, &lab, &checks).await;
    finish(checks, cleanup)
}
