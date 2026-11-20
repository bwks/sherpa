use anyhow::{Context, Result, ensure};
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{
    CreateContainerOptions, RemoveImageOptions, StartContainerOptions, TagImageOptions,
};
use db::DefaultRequest;
use reqwest::{Client, Method, StatusCode, header};
use serde_json::{Value, json};
use shared::error::RpcErrorCode;
use sherpad::auth::cookies::AUTH_COOKIE_NAME;
use std::time::Duration;

use shared::data::{
    ContainerSettings, KindSettings, ModelKind, NodeConfig, NodeKind, NodeModel, ZtpSettings,
};

use crate::helpers::test_server::TestServer;
use crate::helpers::ws_client::TestWsClient;

/// The `arista_eos` versions the image path tests seed: one virtual machine, one container.
const EOS_VM_VERSION: &str = "4.33.0F";
const EOS_CONTAINER_VERSION: &str = "4.34.0F";

// ── Image Scan ──

/// The status the scan in `response` gave `model` at `version`, if it found it.
fn scanned_status<'a>(response: &'a Value, model: &str, version: &str) -> Option<&'a str> {
    response["result"]["scanned"]
        .as_array()?
        .iter()
        .find(|image| image["model"] == model && image["version"] == version)?["status"]
        .as_str()
}

/// Scan local Docker for the `generic` container's busybox tags, recording them unless `dry_run`.
async fn scan_busybox(ws: &mut TestWsClient, token: &str, dry_run: bool) -> Result<Value> {
    ws.rpc_call(
        "image.scan",
        json!({
            "token": token,
            "kind": "container",
            "dry_run": dry_run,
            "model": "generic",
            "repo": "busybox",
        }),
    )
    .await
}

/// The `generic` container image recorded at busybox's version 1.36, if there is one.
async fn recorded_busybox(server: &TestServer) -> Result<Option<NodeConfig>> {
    db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::Generic,
        &NodeKind::Container,
        "1.36",
    )
    .await
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_scan_002_a_version_already_recorded_is_reported_already_exists() -> Result<()> {
    container::pull_image("busybox", "1.36", |_| {}).await?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    scan_busybox(&mut ws, &token, false).await?;

    let rescanned = scan_busybox(&mut ws, &token, false).await?;

    assert_eq!(
        scanned_status(&rescanned, "generic", "1.36"),
        Some("already_exists"),
        "{rescanned:?}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_scan_003_a_dry_run_reports_what_would_be_recorded_and_records_nothing() -> Result<()>
{
    container::pull_image("busybox", "1.36", |_| {}).await?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let dry_run = scan_busybox(&mut ws, &token, true).await?;

    assert_eq!(
        scanned_status(&dry_run, "generic", "1.36").map(|s| s.starts_with("would_import")),
        Some(true),
        "{dry_run:?}"
    );
    assert!(
        recorded_busybox(&server).await?.is_none(),
        "a dry run records nothing"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_scan_004_a_model_and_repo_record_each_local_tag_of_the_repo_with_the_repo()
-> Result<()> {
    container::pull_image("busybox", "1.36", |_| {}).await?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let scanned = scan_busybox(&mut ws, &token, false).await?;

    assert_eq!(
        scanned_status(&scanned, "generic", "1.36").map(|s| s.starts_with("imported")),
        Some(true),
        "{scanned:?}"
    );
    let record = recorded_busybox(&server)
        .await?
        .context("the scan should record busybox:1.36 for the generic container")?;
    assert_eq!(record.container_repo(None).as_deref(), Some("busybox"));

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_default_011_a_scan_makes_the_first_version_it_finds_the_default_and_a_dry_run_says_so()
-> Result<()> {
    container::pull_image("busybox", "1.36", |_| {}).await?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let dry_run = scan_busybox(&mut ws, &token, true).await?;
    let scanned = scan_busybox(&mut ws, &token, false).await?;

    assert_eq!(
        scanned_status(&dry_run, "generic", "1.36"),
        Some("would_import (default)"),
        "{dry_run:?}"
    );
    assert_eq!(
        scanned_status(&scanned, "generic", "1.36"),
        Some("imported (default)"),
        "{scanned:?}"
    );
    let record = recorded_busybox(&server)
        .await?
        .context("the scan should record busybox:1.36 for the generic container")?;
    assert!(record.default);

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_scan_005_each_container_model_gets_its_built_in_repos_tags_and_those_without_one_are_skipped()
-> Result<()> {
    container::pull_image("nats", "2.10-alpine", |_| {}).await?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call("image.scan", json!({ "token": token, "kind": "container" }))
        .await?;

    assert_eq!(
        scanned_status(&response, "nats_bus", "2.10-alpine").map(|s| s.starts_with("imported")),
        Some(true),
        "{response:?}"
    );
    let record = db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::NatsBus,
        &NodeKind::Container,
        "2.10-alpine",
    )
    .await?
    .context("the scan should record nats:2.10-alpine for nats_bus")?;
    assert_eq!(record.container_repo(None).as_deref(), Some("nats"));
    let skipped: Vec<&str> = response["result"]["skipped"]
        .as_array()
        .context("the scan should list skipped models")?
        .iter()
        .filter_map(|model| model["model"].as_str())
        .collect();
    assert_eq!(skipped, ["mysql_db", "postgresql_db", "generic"]);

    Ok(())
}

/// The versions a dry-run scan of `repo` for a `generic` container would record.
async fn dry_run_versions(ws: &mut TestWsClient, token: &str, repo: &str) -> Result<Vec<String>> {
    let response = ws
        .rpc_call(
            "image.scan",
            json!({
                "token": token,
                "kind": "container",
                "dry_run": true,
                "model": "generic",
                "repo": repo,
            }),
        )
        .await?;
    let scanned = response["result"]["scanned"]
        .as_array()
        .with_context(|| format!("the scan of {repo} should list what it found: {response:?}"))?;
    Ok(scanned
        .iter()
        .filter_map(|image| image["version"].as_str().map(ToString::to_string))
        .collect())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_scan_006_a_docker_hub_repo_is_matched_by_its_short_and_full_name_only() -> Result<()>
{
    container::pull_image("busybox", "1.36", |_| {}).await?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    for repo in ["busybox", "docker.io/library/busybox"] {
        let versions = dry_run_versions(&mut ws, &token, repo).await?;
        assert!(
            versions.contains(&"1.36".to_string()),
            "{repo}: {versions:?}"
        );
    }
    for repo in ["docker.io/busybox", "index.docker.io/library/busybox"] {
        let versions = dry_run_versions(&mut ws, &token, repo).await?;
        assert!(versions.is_empty(), "{repo}: {versions:?}");
    }

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_scan_007_a_repo_never_matches_a_repo_it_is_the_start_or_end_of() -> Result<()> {
    const REPO: &str = "sherpa-test-image-scan";
    container::pull_image("busybox", "1.36", |_| {}).await?;
    let docker = container::docker_connection()?;
    let others = [
        "sherpa-test-image-scan-box",
        "sherpa-test/sherpa-test-image-scan",
    ];
    for repo in others {
        docker
            .tag_image(
                "busybox:1.36",
                Some(TagImageOptions {
                    repo: Some(repo.to_string()),
                    tag: Some("1".to_string()),
                }),
            )
            .await?;
    }
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let versions = dry_run_versions(&mut ws, &token, REPO).await;

    for repo in others {
        docker
            .remove_image(&format!("{repo}:1"), None::<RemoveImageOptions>, None)
            .await?;
    }
    let versions = versions?;
    assert!(versions.is_empty(), "{REPO}: {versions:?}");

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn image_scan_008_a_repo_with_a_tag_is_refused_saying_how_to_write_it() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call(
            "image.scan",
            json!({
                "token": token,
                "kind": "container",
                "model": "mysql_db",
                "repo": "mysql:8.4",
            }),
        )
        .await?;

    let error = response["error"].to_string();
    assert!(error.contains("without a tag"), "{response:?}");

    Ok(())
}

// ── Image List ──

/// Record an `ubuntu_linux` virtual machine image at `version`, the default when `default`.
async fn seed_ubuntu_image(server: &TestServer, version: &str, default: bool) -> Result<()> {
    let mut image = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
    version.clone_into(&mut image.version);
    image.default = default;
    db::upsert_node_image(&server.db, image, DefaultRequest::Keep).await?;
    Ok(())
}

#[tokio::test]
async fn image_list_001_list_gives_each_images_model_kind_version_and_default() -> Result<()> {
    let server = TestServer::start().await?;
    seed_ubuntu_image(&server, "24.04", true).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws.rpc_call("image.list", json!({ "token": token })).await?;

    assert_eq!(
        response["result"],
        json!({
            "images": [{
                "model": "ubuntu_linux",
                "kind": "virtual_machine",
                "version": "24.04",
                "default": true,
            }],
            "total": 1,
        }),
        "{response}"
    );

    Ok(())
}

// ── Image Show ──

#[tokio::test]
async fn image_list_003_show_without_a_version_gives_the_default_versions_settings() -> Result<()> {
    let server = TestServer::start().await?;
    seed_ubuntu_image(&server, "24.04", true).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call(
            "image.show",
            json!({
                "token": token,
                "model": "ubuntu_linux",
            }),
        )
        .await?;

    assert_eq!(
        response["result"]["image"]["version"], "24.04",
        "{response}"
    );

    Ok(())
}

// ── Image Delete ──

/// Tag busybox as `repo:1` in Docker and record it as a `generic` container image, version 1.
async fn recorded_test_image(server: &TestServer, repo: &str) -> Result<()> {
    container::pull_image("busybox", "1.36", |_| {}).await?;
    container::docker_connection()?
        .tag_image(
            "busybox:1.36",
            Some(TagImageOptions {
                repo: Some(repo.to_string()),
                tag: Some("1".to_string()),
            }),
        )
        .await?;
    let image = NodeConfig {
        version: "1".to_owned(),
        default: true,
        settings: KindSettings::Container(ContainerSettings {
            repo: Some(repo.to_owned()),
            ztp: ZtpSettings::default(),
        }),
        ..ModelKind::new(NodeModel::Generic, Some(NodeKind::Container))?.node_config()
    };
    db::upsert_node_image(&server.db, image, DefaultRequest::Keep).await?;
    Ok(())
}

/// Delete the `generic` container image, version 1.
async fn delete_test_image(ws: &mut TestWsClient, token: &str) -> Result<Value> {
    ws.rpc_call(
        "image.delete",
        json!({
            "token": token,
            "model": "generic",
            "kind": "container",
            "version": "1",
        }),
    )
    .await
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_delete_001_deleting_a_container_image_removes_its_record_and_its_tag_from_docker()
-> Result<()> {
    const REPO: &str = "sherpa-test-image-delete";
    let server = TestServer::start().await?;
    recorded_test_image(&server, REPO).await?;
    let docker = container::docker_connection()?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = delete_test_image(&mut ws, &token).await?;

    assert_eq!(response["result"]["image_deleted"], true, "{response:?}");
    assert!(!container::image_exists(&docker, &format!("{REPO}:1")).await?);
    // busybox itself, another tag of the same image, is left alone.
    assert!(container::image_exists(&docker, "busybox:1.36").await?);
    let record = db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::Generic,
        &NodeKind::Container,
        "1",
    )
    .await?;
    assert!(record.is_none(), "the record is deleted");
    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_delete_002_a_container_image_a_running_container_runs_is_not_deleted() -> Result<()>
{
    const REPO: &str = "sherpa-test-image-in-use";
    const CONTAINER: &str = "sherpa-test-image-in-use-runner";
    let server = TestServer::start().await?;
    recorded_test_image(&server, REPO).await?;
    let docker = container::docker_connection()?;
    docker
        .create_container(
            Some(CreateContainerOptions {
                name: Some(CONTAINER.to_owned()),
                ..Default::default()
            }),
            ContainerCreateBody {
                image: Some(format!("{REPO}:1")),
                cmd: Some(vec!["sleep".to_owned(), "300".to_owned()]),
                ..Default::default()
            },
        )
        .await?;
    docker
        .start_container(CONTAINER, None::<StartContainerOptions>)
        .await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let refused = delete_test_image(&mut ws, &token).await;
    container::remove_container(&docker, CONTAINER).await?;
    let refused = refused?;

    let message = refused["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(&format!("in use by running container(s): {CONTAINER}")),
        "{refused:?}"
    );
    assert!(container::image_exists(&docker, &format!("{REPO}:1")).await?);
    let record = db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::Generic,
        &NodeKind::Container,
        "1",
    )
    .await?;
    assert!(record.is_some(), "the record is kept");

    // Once the container is gone, the image is deleted.
    let response = delete_test_image(&mut ws, &token).await?;
    assert_eq!(response["result"]["image_deleted"], true, "{response:?}");
    Ok(())
}

// ── Image Set Default ──

#[tokio::test]
async fn image_default_004_set_default_moves_the_default_to_the_version_it_names() -> Result<()> {
    let server = TestServer::start().await?;
    seed_ubuntu_image(&server, "22.04", true).await?;
    seed_ubuntu_image(&server, "24.04", false).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call(
            "image.set_default",
            json!({
                "token": token,
                "model": "ubuntu_linux",
                "version": "24.04",
            }),
        )
        .await?;

    assert!(
        response.get("result").is_some(),
        "set_default should succeed: {response:?}"
    );
    let default = db::find_default_node_image(
        &server.db,
        &NodeModel::UbuntuLinux,
        &NodeKind::VirtualMachine,
    )
    .await?
    .context("ubuntu_linux should have a default image")?;
    assert_eq!(default.version, "24.04");

    Ok(())
}

// ── Image Import (VM) ──

#[tokio::test]
async fn image_import_002_a_source_file_that_does_not_exist_is_refused_and_nothing_is_recorded()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let (_statuses, response) = ws
        .rpc_call_streaming(
            "image.import",
            json!({
                "token": token,
                "model": "ubuntu_linux",
                "version": "99.99",
                "src": "/nonexistent/path/virtioa.qcow2",
            }),
            Duration::from_secs(30),
        )
        .await?;

    assert!(
        response["error"]
            .to_string()
            .contains("Source file does not exist: /nonexistent/path/virtioa.qcow2"),
        "importing nonexistent file should fail: {response:?}"
    );
    let record = db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::UbuntuLinux,
        &NodeKind::VirtualMachine,
        "99.99",
    )
    .await?;
    assert!(record.is_none(), "a failed import recorded the image");

    Ok(())
}

// ── Image Pull (Container) ──

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn image_pull_001_a_pull_brings_the_image_into_docker_and_records_it() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Pull nokia srlinux (already available locally as ghcr.io/nokia/srlinux:latest)
    let (_statuses, response) = ws
        .rpc_call_streaming(
            "image.pull",
            json!({
                "token": token,
                "model": "nokia_srlinux",
                "repo": "ghcr.io/nokia/srlinux",
                "tag": "latest",
            }),
            Duration::from_secs(120),
        )
        .await?;

    assert!(
        response.get("result").is_some(),
        "image.pull should succeed: {response:?}"
    );

    Ok(())
}

// ── Non-Admin Access Denied ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn image_access_002_scan_is_refused_to_a_user_who_is_not_an_admin() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;

    // Create non-admin user
    let _r = ws
        .rpc_call(
            "user.create",
            json!({
                "token": admin_token,
                "username": "imguser",
                "password": "ImgUser123!",
                "is_admin": false,
            }),
        )
        .await?;

    let user_token = ws.login("imguser", "ImgUser123!").await?;

    // image.scan is streaming, but non-admin should get access denied
    let (_s, response) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": user_token }),
            Duration::from_secs(10),
        )
        .await?;

    assert_eq!(
        response["error"]["code"],
        json!(RpcErrorCode::AccessDenied),
        "non-admin should be denied image.scan: {response}"
    );

    Ok(())
}

// ── Image paths name the model and its kind ──

/// Seed one default `arista_eos` image of each kind, each with its own version.
async fn seed_arista_eos_images(server: &TestServer) -> Result<()> {
    for (kind, version) in [
        (NodeKind::VirtualMachine, EOS_VM_VERSION),
        (NodeKind::Container, EOS_CONTAINER_VERSION),
    ] {
        let mut image = ModelKind::new(NodeModel::AristaEos, Some(kind))?.node_config();
        version.clone_into(&mut image.version);
        image.default = true;
        db::upsert_node_image(&server.db, image, DefaultRequest::Keep).await?;
    }
    Ok(())
}

/// Send `method path` to `server` as `token`, by bearer token for the API and by session
/// cookie for the web pages, and return the status and body.
async fn request(
    server: &TestServer,
    method: Method,
    path: &str,
    token: &str,
) -> Result<(StatusCode, String)> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .request(method, format!("http://{}{path}", server.addr))
        .bearer_auth(token)
        .header(header::COOKIE, format!("{AUTH_COOKIE_NAME}={token}"))
        .send()
        .await?;
    Ok((response.status(), response.text().await?))
}

#[tokio::test]
async fn image_list_005_each_kind_of_a_model_is_shown_made_default_and_edited_under_that_kind()
-> Result<()> {
    let server = TestServer::start().await?;
    seed_arista_eos_images(&server).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    for (kind, version, other_version) in [
        ("virtual_machine", EOS_VM_VERSION, EOS_CONTAINER_VERSION),
        ("container", EOS_CONTAINER_VERSION, EOS_VM_VERSION),
    ] {
        let path = format!("/api/v1/images/arista_eos/{kind}");
        let (status, body) = request(&server, Method::GET, &path, &token).await?;
        ensure!(status == StatusCode::OK, "GET {path}: {status} {body}");
        let shown: Value = serde_json::from_str(&body)?;
        ensure!(
            shown["image"]["kind"] == kind && shown["image"]["version"] == version,
            "GET {path} showed another image: {body}"
        );

        let path = format!("/api/v1/images/arista_eos/{kind}?version={version}");
        let (status, body) = request(&server, Method::GET, &path, &token).await?;
        ensure!(status == StatusCode::OK, "GET {path}: {status} {body}");

        let path = format!("/api/v1/images/arista_eos/{kind}/{version}/default");
        let (status, body) = request(&server, Method::POST, &path, &token).await?;
        ensure!(status == StatusCode::OK, "POST {path}: {status} {body}");
        let set: Value = serde_json::from_str(&body)?;
        ensure!(set["kind"] == kind, "POST {path} set another kind: {body}");

        for page in [
            "",
            "/edit",
            &format!("/{version}"),
            &format!("/{version}/edit"),
        ] {
            let path = format!("/admin/images/arista_eos/{kind}{page}");
            let (status, body) = request(&server, Method::GET, &path, &token).await?;
            ensure!(status == StatusCode::OK, "GET {path}: {status}");
            ensure!(body.contains(version), "GET {path} shows another version");
        }
        let path = format!("/admin/images/arista_eos/{kind}/versions");
        let (status, body) = request(&server, Method::GET, &path, &token).await?;
        ensure!(status == StatusCode::OK, "GET {path}: {status}");
        ensure!(
            body.contains(version) && !body.contains(other_version),
            "GET {path} lists another kind's version"
        );
    }

    // The list has a row for each kind, each linking to its own pages.
    let (status, body) = request(&server, Method::GET, "/admin/images", &token).await?;
    ensure!(status == StatusCode::OK, "GET /admin/images: {status}");
    for kind in ["virtual_machine", "container"] {
        ensure!(
            body.contains(&format!(r#"href="/admin/images/arista_eos/{kind}""#)),
            "/admin/images has no row for {kind}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn image_default_010_making_a_container_version_the_default_leaves_the_virtual_machines()
-> Result<()> {
    let server = TestServer::start().await?;
    seed_arista_eos_images(&server).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let path = format!("/api/v1/images/arista_eos/container/{EOS_CONTAINER_VERSION}/default");
    let (status, body) = request(&server, Method::POST, &path, &token).await?;
    ensure!(status == StatusCode::OK, "POST {path}: {status} {body}");

    for (kind, version) in [
        ("virtual_machine", EOS_VM_VERSION),
        ("container", EOS_CONTAINER_VERSION),
    ] {
        let path = format!("/api/v1/images/arista_eos/{kind}");
        let (status, body) = request(&server, Method::GET, &path, &token).await?;
        ensure!(status == StatusCode::OK, "GET {path}: {status} {body}");
        let shown: Value = serde_json::from_str(&body)?;
        ensure!(
            shown["image"]["kind"] == kind && shown["image"]["version"] == version,
            "GET {path} showed another default: {body}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn image_list_006_a_kind_the_model_does_not_take_is_refused_as_a_bad_request() -> Result<()> {
    let server = TestServer::start().await?;
    seed_arista_eos_images(&server).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    for (method, path) in [
        (
            Method::GET,
            "/api/v1/images/arista_eos/unikernel".to_owned(),
        ),
        (
            Method::DELETE,
            format!("/api/v1/images/arista_eos/unikernel/{EOS_VM_VERSION}"),
        ),
        (
            Method::POST,
            format!("/api/v1/images/arista_eos/unikernel/{EOS_VM_VERSION}/default"),
        ),
        (Method::GET, "/admin/images/arista_eos/unikernel".to_owned()),
        (
            Method::GET,
            "/admin/images/arista_eos/unikernel/versions".to_owned(),
        ),
    ] {
        let (status, body) = request(&server, method.clone(), &path, &token).await?;
        ensure!(
            status == StatusCode::BAD_REQUEST && body.contains("is not a unikernel"),
            "{method} {path}: {status} {body}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn image_list_007_a_path_whose_kind_is_not_a_kind_is_refused_as_a_bad_request() -> Result<()>
{
    let server = TestServer::start().await?;
    seed_arista_eos_images(&server).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // A page path without a kind reads its version or `edit` as the kind.
    for path in [
        format!("/admin/images/arista_eos/{EOS_VM_VERSION}"),
        "/admin/images/arista_eos/edit".to_owned(),
        "/admin/images/arista_eos/versions".to_owned(),
    ] {
        let (status, body) = request(&server, Method::GET, &path, &token).await?;
        ensure!(
            status == StatusCode::BAD_REQUEST && body.contains("Unknown node kind"),
            "GET {path}: {status} {body}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_old_model_name_in_an_image_path_is_refused_naming_its_replacement() -> Result<()> {
    let server = TestServer::start().await?;
    seed_arista_eos_images(&server).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    for path in [
        "/api/v1/images/arista_ceos/container",
        "/admin/images/arista_ceos/container",
    ] {
        let (status, body) = request(&server, Method::GET, path, &token).await?;
        ensure!(
            status == StatusCode::BAD_REQUEST && body.contains("renamed in 0.7.0"),
            "GET {path}: {status} {body}"
        );
    }
    Ok(())
}
