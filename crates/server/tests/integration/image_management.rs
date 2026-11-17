use anyhow::{Context, Result, ensure};
use bollard::query_parameters::{RemoveImageOptions, TagImageOptions};
use db::DefaultRequest;
use reqwest::{Client, Method, StatusCode, header};
use serde_json::{Value, json};
use sherpad::auth::cookies::AUTH_COOKIE_NAME;
use std::time::Duration;

use shared::data::{ModelKind, NodeKind, NodeModel};

use crate::helpers::test_server::TestServer;
use crate::helpers::ws_client::TestWsClient;

/// The `arista_eos` versions the image path tests seed: one virtual machine, one container.
const EOS_VM_VERSION: &str = "4.33.0F";
const EOS_CONTAINER_VERSION: &str = "4.34.0F";

// ── Image Scan ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_scan_discovers_images() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let (_statuses, response) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;

    assert!(
        response.get("result").is_some(),
        "image.scan should succeed: {response:?}"
    );
    // Status messages are emitted per discovered image; OK if none found on a clean env

    Ok(())
}

/// The status the scan in `response` gave `model` at `version`, if it found it.
fn scanned_status<'a>(response: &'a Value, model: &str, version: &str) -> Option<&'a str> {
    response["result"]["scanned"]
        .as_array()?
        .iter()
        .find(|image| image["model"] == model && image["version"] == version)?["status"]
        .as_str()
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn test_image_scan_records_a_local_image_for_a_model_and_repo() -> Result<()> {
    container::pull_image("busybox", "1.36", |_| {}).await?;
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    let scan = |dry_run: bool| {
        json!({
            "token": token,
            "kind": "container",
            "dry_run": dry_run,
            "model": "generic",
            "repo": "busybox",
        })
    };

    let dry_run = ws.rpc_call("image.scan", scan(true)).await?;

    assert_eq!(
        scanned_status(&dry_run, "generic", "1.36"),
        Some("would_import (default)"),
        "{dry_run:?}"
    );
    let unrecorded = db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::Generic,
        &NodeKind::Container,
        "1.36",
    )
    .await?;
    assert!(unrecorded.is_none(), "a dry run records nothing");

    let scanned = ws.rpc_call("image.scan", scan(false)).await?;

    assert_eq!(
        scanned_status(&scanned, "generic", "1.36"),
        Some("imported (default)"),
        "{scanned:?}"
    );
    let record = db::find_node_image_by_model_kind_version(
        &server.db,
        &NodeModel::Generic,
        &NodeKind::Container,
        "1.36",
    )
    .await?
    .context("the scan should record busybox:1.36 for the generic container")?;
    assert_eq!(record.repo.as_deref(), Some("busybox"));
    assert!(record.default);

    let rescanned = ws.rpc_call("image.scan", scan(false)).await?;

    assert_eq!(
        scanned_status(&rescanned, "generic", "1.36"),
        Some("already_exists"),
        "{rescanned:?}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn test_image_scan_matches_a_built_in_repo_and_skips_models_without_one() -> Result<()> {
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
    assert_eq!(record.repo.as_deref(), Some("nats"));
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
async fn test_image_scan_matches_a_docker_hub_repo_by_its_short_and_full_name_only() -> Result<()> {
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
async fn test_image_scan_does_not_match_a_repo_by_its_prefix() -> Result<()> {
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
async fn test_image_scan_refuses_a_repo_with_a_tag() -> Result<()> {
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

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_list_returns_images() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // First scan to register images
    let (_s, _r) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;

    // List all images
    let response = ws.rpc_call("image.list", json!({ "token": token })).await?;

    assert!(
        response.get("result").is_some(),
        "image.list should succeed: {response:?}"
    );

    Ok(())
}

// ── Image Show ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_show_returns_config() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Scan first
    let (_s, _r) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;

    // Show a specific model — use ubuntu_linux which should be on disk
    let response = ws
        .rpc_call(
            "image.show",
            json!({
                "token": token,
                "model": "ubuntu_linux",
            }),
        )
        .await?;

    // If image was found, result should have config details
    // image.show returns ShowImageResponse { image: NodeConfig }
    if response.get("result").is_some() {
        let result = response.get("result").unwrap();
        assert!(
            result.get("image").is_some(),
            "image.show result should have 'image' field: {result:?}"
        );
        // The image's ZTP group has only its method: no login settings, and no password flag.
        let ztp = result["image"]["ztp"].as_object().unwrap();
        assert_eq!(ztp.keys().collect::<Vec<_>>(), ["method"], "{result:?}");
        assert!(result.get("ztp_password_set").is_none(), "{result:?}");
    }

    Ok(())
}

// ── Image Set Default ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_set_default() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Scan first
    let (_s, _r) = ws
        .rpc_call_streaming(
            "image.scan",
            json!({ "token": token }),
            Duration::from_secs(60),
        )
        .await?;

    // List images to get a model and version
    let list_resp = ws.rpc_call("image.list", json!({ "token": token })).await?;

    if let Some(result) = list_resp.get("result")
        && let Some(images) = result.as_array()
        && let Some(first) = images.first()
    {
        let model = first.get("model").and_then(|v| v.as_str()).unwrap_or("");
        let version = first.get("version").and_then(|v| v.as_str()).unwrap_or("");

        if !model.is_empty() && !version.is_empty() {
            let response = ws
                .rpc_call(
                    "image.set_default",
                    json!({
                        "token": token,
                        "model": model,
                        "version": version,
                    }),
                )
                .await?;

            assert!(
                response.get("result").is_some(),
                "set_default should succeed: {response:?}"
            );
        }
    }

    Ok(())
}

// ── Image Import (VM) ──

#[tokio::test]
#[ignore = "requires a Docker daemon"]
async fn test_image_import_nonexistent_file() -> Result<()> {
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
                "path": "/nonexistent/path/virtioa.qcow2",
            }),
            Duration::from_secs(30),
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "importing nonexistent file should fail: {response:?}"
    );

    Ok(())
}

// ── Image Pull (Container) ──

#[tokio::test]
#[ignore = "requires a Docker daemon and internet access to pull an image"]
async fn test_image_pull_container() -> Result<()> {
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
async fn test_image_scan_admin_only() -> Result<()> {
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

    assert!(
        response.get("error").is_some(),
        "non-admin should be denied image.scan"
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
async fn the_image_paths_resolve_the_image_of_each_kind_of_arista_eos() -> Result<()> {
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
async fn an_image_path_with_a_kind_the_model_does_not_take_is_refused() -> Result<()> {
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
async fn the_old_image_paths_and_model_names_are_refused() -> Result<()> {
    let server = TestServer::start().await?;
    seed_arista_eos_images(&server).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // The paths without a kind are gone.
    for (method, path) in [
        (Method::GET, "/api/v1/images/arista_eos".to_owned()),
        (
            Method::DELETE,
            format!("/api/v1/images/arista_eos/{EOS_VM_VERSION}"),
        ),
        (
            Method::POST,
            format!("/api/v1/images/arista_eos/{EOS_VM_VERSION}/default"),
        ),
        (Method::GET, "/admin/images/arista_eos".to_owned()),
    ] {
        let (status, body) = request(&server, method.clone(), &path, &token).await?;
        ensure!(
            matches!(
                status,
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            "{method} {path}: {status} {body}"
        );
    }
    // An old page path reads its version or `edit` as the kind.
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
    // An old model name is refused naming its replacement.
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
