mod helpers;

use anyhow::{Context, Result, ensure};
use reqwest::{Client, Response, StatusCode, header};
use serde_json::{Value, json};
use shared::data::DbUser;
use shared::error::RpcErrorCode;
use shared::konst::SSH_KEYS_DOCS_URL;
use sherpad::auth::cookies::AUTH_COOKIE_NAME;
use std::time::Duration;

use helpers::test_server::{TEST_ADMIN_PASSWORD, TestServer};
use helpers::ws_client::TestWsClient;

const RPC_TIMEOUT: Duration = Duration::from_secs(60);

/// A logged-in caller in the authorization matrix.
struct Caller {
    username: &'static str,
    token: String,
    is_admin: bool,
}

/// IPv4-only lab networks for tests that seed lab records directly.
fn ipv4_lab_networks(index: usize) -> db::LabNetworks {
    db::LabNetworks {
        loopback_network: format!("127.126.{index}.0/24"),
        management_network: format!("10.201.{index}.0/24"),
        gateway_ipv4: format!("10.201.{index}.1"),
        router_ipv4: format!("10.201.{index}.2"),
        management_network_v6: None,
        loopback_network_v6: None,
        gateway_ipv6: None,
        router_ipv6: None,
    }
}

/// Seed a lab record with no nodes or host resources.
async fn seed_lab(server: &TestServer, owner: &DbUser, index: usize) -> Result<String> {
    let lab_id = format!("{:08x}", rand::random::<u32>());
    db::create_lab(
        &server.db,
        "auth-matrix",
        &lab_id,
        owner,
        &ipv4_lab_networks(index),
    )
    .await?;
    Ok(lab_id)
}

/// Remove a seeded lab record if an authorized destroy did not already remove it.
async fn remove_lab(server: &TestServer, lab_id: &str) -> Result<()> {
    if db::find_lab(&server.db, lab_id).await?.is_some() {
        db::delete_lab(&server.db, lab_id).await?;
    }
    Ok(())
}

/// Assert an RPC call was refused exactly when the caller is not allowed.
async fn check_rpc(
    ws: &mut TestWsClient,
    method: &str,
    params: Value,
    allowed: bool,
    label: &str,
) -> Result<()> {
    let (statuses, response) = ws.rpc_call_streaming(method, params, RPC_TIMEOUT).await?;
    let denied = response["error"]["code"] == json!(RpcErrorCode::AccessDenied);
    ensure!(
        denied != allowed,
        "{label}: expected allowed={allowed}, got {response}"
    );
    if !allowed {
        ensure!(
            statuses.is_empty(),
            "{label}: refused request streamed progress: {statuses:?}"
        );
    }
    Ok(())
}

/// Assert an HTTP response was refused exactly when the caller is not allowed.
fn check_http(response: &Response, allowed: bool, label: &str) -> Result<()> {
    let denied = response.status() == StatusCode::FORBIDDEN;
    ensure!(
        denied != allowed,
        "{label}: expected allowed={allowed}, got {}",
        response.status()
    );
    Ok(())
}

fn cookie(token: &str) -> String {
    format!("{AUTH_COOKIE_NAME}={token}")
}

/// Log in alice and bob (regular users) and dana (a personal admin account).
///
/// The built-in `admin` account cannot start lab work, so the matrix uses a personal admin;
/// `built_in_admin_cannot_start_lab_work_on_any_transport` covers `admin` itself.
async fn login_callers(
    server: &TestServer,
    ws: &mut TestWsClient,
) -> Result<(Vec<Caller>, Vec<DbUser>)> {
    let mut owners = Vec::new();
    for (username, is_admin) in [("dana", true), ("alice", false), ("bob", false)] {
        owners.push(
            db::create_user(
                &server.db,
                username.to_owned(),
                TEST_ADMIN_PASSWORD,
                is_admin,
                vec![],
            )
            .await?,
        );
    }
    let callers = vec![
        Caller {
            username: "alice",
            token: ws.login("alice", TEST_ADMIN_PASSWORD).await?,
            is_admin: false,
        },
        Caller {
            username: "bob",
            token: ws.login("bob", TEST_ADMIN_PASSWORD).await?,
            is_admin: false,
        },
        Caller {
            username: "dana",
            token: ws.login("dana", TEST_ADMIN_PASSWORD).await?,
            is_admin: true,
        },
    ];
    Ok((callers, owners))
}

fn token_of(callers: &[Caller], username: &str) -> Result<String> {
    callers
        .iter()
        .find(|caller| caller.username == username)
        .map(|caller| caller.token.clone())
        .context("caller missing")
}

async fn list_labs(http: &Client, base: &str, token: &str, query: &str) -> Result<Response> {
    http.get(format!("{base}/api/v1/labs{query}"))
        .bearer_auth(token)
        .send()
        .await
        .context("lab list request failed")
}

fn http_client() -> Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

#[tokio::test]
#[ignore = "requires the disposable SurrealDB test database, Docker and libvirt"]
async fn lab_operations_allow_owner_and_admins_on_all_transports() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let (callers, owners) = login_callers(&server, &mut ws).await?;
    let http = http_client()?;
    let base = format!("http://{}", server.addr);

    let mut index = 0;
    // Exercise both regular-user and admin-owned labs.
    for owner in owners.iter().filter(|owner| owner.username != "bob") {
        for caller in &callers {
            index += 1;
            let lab_id = seed_lab(&server, owner, index).await?;
            let allowed = caller.is_admin || caller.username == owner.username;
            let who = format!("{} on {}'s lab", caller.username, owner.username);
            let token = caller.token.as_str();

            // Read-only and node power operations.
            for method in ["inspect", "download", "down", "resume"] {
                let params = json!({ "token": token, "lab_id": lab_id });
                check_rpc(
                    &mut ws,
                    method,
                    params,
                    allowed,
                    &format!("RPC {method}, {who}"),
                )
                .await?;
            }
            let params = json!({ "token": token, "lab_id": lab_id, "link_index": 0 });
            check_rpc(
                &mut ws,
                "link.update_impairment",
                params,
                allowed,
                &format!("RPC link.update_impairment, {who}"),
            )
            .await?;

            let response = http
                .get(format!("{base}/api/v1/labs/{lab_id}"))
                .bearer_auth(token)
                .send()
                .await?;
            check_http(&response, allowed, &format!("REST inspect, {who}"))?;
            for action in ["down", "resume"] {
                let response = http
                    .post(format!("{base}/api/v1/labs/{lab_id}/{action}"))
                    .bearer_auth(token)
                    .json(&json!({}))
                    .send()
                    .await?;
                check_http(&response, allowed, &format!("REST {action}, {who}"))?;
            }
            let response = http
                .post(format!("{base}/api/v1/labs/{lab_id}/links/0/impairment"))
                .bearer_auth(token)
                .json(&json!({ "lab_id": lab_id, "link_index": 0 }))
                .send()
                .await?;
            check_http(&response, allowed, &format!("REST impairment, {who}"))?;

            for path in [
                format!("/labs/{lab_id}"),
                format!("/labs/{lab_id}/nodes"),
                format!("/labs/{lab_id}/nodes/vm"),
                format!("/labs/{lab_id}/destroy/confirm"),
                format!("/labs/{lab_id}/download"),
            ] {
                let response = http
                    .get(format!("{base}{path}"))
                    .header(header::COOKIE, cookie(token))
                    .send()
                    .await?;
                check_http(&response, allowed, &format!("web GET {path}, {who}"))?;
            }
            for path in [
                format!("/labs/{lab_id}/stop"),
                format!("/labs/{lab_id}/start"),
                format!("/labs/{lab_id}/nodes/vm/stop"),
                format!("/labs/{lab_id}/nodes/vm/start"),
                // Creates a destroy job without running it.
                format!("/labs/{lab_id}/destroy"),
            ] {
                let response = http
                    .post(format!("{base}{path}"))
                    .header(header::COOKIE, cookie(token))
                    .send()
                    .await?;
                check_http(&response, allowed, &format!("web POST {path}, {who}"))?;
            }

            // Destroy last: an allowed destroy removes the lab.
            let params = json!({ "token": token, "lab_id": lab_id });
            check_rpc(
                &mut ws,
                "destroy",
                params,
                allowed,
                &format!("RPC destroy, {who}"),
            )
            .await?;
            if !allowed {
                let response = http
                    .delete(format!("{base}/api/v1/labs/{lab_id}"))
                    .bearer_auth(token)
                    .send()
                    .await?;
                check_http(&response, allowed, &format!("REST destroy, {who}"))?;
                ensure!(
                    db::find_lab(&server.db, &lab_id).await?.is_some(),
                    "{who}: refused destroy removed the lab"
                );
            }
            remove_lab(&server, &lab_id).await?;
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires the disposable SurrealDB test database"]
async fn admin_destroy_over_rest_reaches_the_service() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let (callers, owners) = login_callers(&server, &mut ws).await?;
    let admin = callers
        .iter()
        .find(|caller| caller.is_admin)
        .context("admin caller missing")?;
    let alice = owners
        .iter()
        .find(|owner| owner.username == "alice")
        .context("alice missing")?;
    let lab_id = seed_lab(&server, alice, 1).await?;

    let response = http_client()?
        .delete(format!("http://{}/api/v1/labs/{lab_id}", server.addr))
        .bearer_auth(&admin.token)
        .send()
        .await?;
    ensure!(
        response.status() == StatusCode::OK,
        "Admin REST destroy was refused: {}",
        response.status()
    );
    let body = response.text().await?;
    ensure!(
        !body.contains("owned by another user"),
        "Admin REST destroy was refused by the service: {body}"
    );
    remove_lab(&server, &lab_id).await
}

#[tokio::test]
#[ignore = "requires the disposable SurrealDB test database"]
async fn job_stream_is_served_only_to_its_creator_or_an_admin() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let (callers, owners) = login_callers(&server, &mut ws).await?;
    let http = http_client()?;
    let base = format!("http://{}", server.addr);
    let alice_owner = owners
        .iter()
        .find(|owner| owner.username == "alice")
        .context("alice missing")?;
    let alice = token_of(&callers, "alice")?;
    let bob = token_of(&callers, "bob")?;
    let lab_id = seed_lab(&server, alice_owner, 1).await?;

    let response = http
        .post(format!("{base}/labs/{lab_id}/destroy"))
        .header(header::COOKIE, cookie(&alice))
        .send()
        .await?;
    let job_url = response
        .headers()
        .get("hx-redirect")
        .and_then(|value| value.to_str().ok())
        .context("destroy did not create a job")?
        .to_owned();

    let response = http
        .get(format!("{base}{job_url}/stream"))
        .header(header::COOKIE, cookie(&bob))
        .send()
        .await?;
    ensure!(
        response.status() == StatusCode::FORBIDDEN,
        "Another user streamed alice's job: {}",
        response.status()
    );
    let response = http
        .get(format!("{base}{job_url}"))
        .header(header::COOKIE, cookie(&bob))
        .send()
        .await?;
    ensure!(
        response.status() == StatusCode::FORBIDDEN,
        "Another user viewed alice's job page: {}",
        response.status()
    );

    // The refused requests must not consume the job.
    let response = http
        .get(format!("{base}{job_url}/stream"))
        .header(header::COOKIE, cookie(&alice))
        .send()
        .await?;
    ensure!(
        response.status() == StatusCode::OK,
        "Job creator could not stream the job: {}",
        response.status()
    );
    let body = response.text().await?;
    ensure!(
        !body.contains("Job not found"),
        "Refused request consumed the job: {body}"
    );
    remove_lab(&server, &lab_id).await
}

#[tokio::test]
#[ignore = "requires the disposable SurrealDB test database"]
async fn lab_listing_requires_login_and_admin_for_other_users() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let (callers, owners) = login_callers(&server, &mut ws).await?;
    let http = http_client()?;
    let base = format!("http://{}", server.addr);
    for (index, owner) in owners.iter().enumerate() {
        seed_lab(&server, owner, index).await?;
    }

    let response = http
        .get(format!("{base}/api/v1/labs?username=alice"))
        .send()
        .await?;
    ensure!(
        response.status() == StatusCode::UNAUTHORIZED,
        "Unauthenticated lab listing was allowed: {}",
        response.status()
    );

    let response = list_labs(&http, &base, &token_of(&callers, "alice")?, "").await?;
    ensure!(response.status() == StatusCode::OK, "{}", response.status());
    let body: Value = response.json().await?;
    ensure!(body["total"] == json!(1), "alice saw other labs: {body}");

    let response = list_labs(&http, &base, &token_of(&callers, "alice")?, "?username=bob").await?;
    ensure!(
        response.status() == StatusCode::FORBIDDEN,
        "A regular user listed another user's labs: {}",
        response.status()
    );

    let response = list_labs(
        &http,
        &base,
        &token_of(&callers, "dana")?,
        "?username=alice",
    )
    .await?;
    ensure!(response.status() == StatusCode::OK, "{}", response.status());
    let body: Value = response.json().await?;
    ensure!(
        body["total"] == json!(1),
        "admin did not get alice's labs: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires the disposable SurrealDB test database, Docker and libvirt"]
async fn built_in_admin_cannot_start_lab_work_on_any_transport() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let (_, owners) = login_callers(&server, &mut ws).await?;
    let admin = ws.login_admin().await?;
    let http = http_client()?;
    let base = format!("http://{}", server.addr);
    let alice = owners
        .iter()
        .find(|owner| owner.username == "alice")
        .context("alice missing")?;
    let lab_id = seed_lab(&server, alice, 1).await?;
    let manifest = json!({ "name": "admin-lab", "nodes": [] });

    for (method, params) in [
        (
            "up",
            json!({ "token": admin, "lab_id": "adm1n001", "manifest": manifest }),
        ),
        ("resume", json!({ "token": admin, "lab_id": lab_id })),
        (
            "redeploy",
            json!({ "token": admin, "lab_id": lab_id, "node_name": "vm", "manifest": manifest }),
        ),
    ] {
        check_rpc(
            &mut ws,
            method,
            params,
            false,
            &format!("RPC {method}, admin"),
        )
        .await?;
    }

    let response = http
        .post(format!("{base}/api/v1/labs"))
        .bearer_auth(&admin)
        .json(&json!({ "lab_id": "adm1n002", "manifest": manifest }))
        .send()
        .await?;
    check_http(&response, false, "REST create, admin")?;
    let response = http
        .post(format!("{base}/api/v1/labs/{lab_id}/resume"))
        .bearer_auth(&admin)
        .json(&json!({}))
        .send()
        .await?;
    check_http(&response, false, "REST resume, admin")?;
    for path in [
        format!("/labs/{lab_id}/start"),
        format!("/labs/{lab_id}/nodes/vm/start"),
    ] {
        let response = http
            .post(format!("{base}{path}"))
            .header(header::COOKIE, cookie(&admin))
            .send()
            .await?;
        check_http(&response, false, &format!("web POST {path}, admin"))?;
    }
    let response = http
        .post(format!("{base}/labs/create"))
        .header(header::COOKIE, cookie(&admin))
        .form(&[("manifest", "name = \"admin-lab\"\nnodes = []\n")])
        .send()
        .await?;
    let body = response.text().await?;
    ensure!(
        body.contains("cannot start, resume or redeploy labs"),
        "web create was not refused for admin: {body}"
    );

    // Clean-up work is still allowed.
    check_rpc(
        &mut ws,
        "down",
        json!({ "token": admin, "lab_id": lab_id }),
        true,
        "RPC down, admin",
    )
    .await?;
    check_rpc(
        &mut ws,
        "destroy",
        json!({ "token": admin, "lab_id": lab_id }),
        true,
        "RPC destroy, admin",
    )
    .await?;
    remove_lab(&server, &lab_id).await
}

#[tokio::test]
#[ignore = "requires the disposable SurrealDB test database and libvirt"]
async fn starting_a_lab_requires_a_registered_ssh_key() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    // login_callers creates users without SSH keys.
    let (callers, _) = login_callers(&server, &mut ws).await?;
    let alice = token_of(&callers, "alice")?;
    let lab_id = format!("{:08x}", rand::random::<u32>());
    let manifest = json!({ "name": "no-key-lab", "nodes": [] });

    let (statuses, response) = ws
        .rpc_call_streaming(
            "up",
            json!({ "token": alice, "lab_id": lab_id, "manifest": manifest }),
            RPC_TIMEOUT,
        )
        .await?;
    let context = response["error"]["context"].as_str().unwrap_or_default();
    ensure!(
        context.contains("no SSH public key") && context.contains(SSH_KEYS_DOCS_URL),
        "up without an SSH key was not refused with guidance: {response} {statuses:?}"
    );
    ensure!(
        db::find_lab(&server.db, &lab_id).await?.is_none(),
        "a refused up created a lab record"
    );
    Ok(())
}
