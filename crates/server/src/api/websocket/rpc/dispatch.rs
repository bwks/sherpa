use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::Message;
use opentelemetry::KeyValue;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use shared::error::RpcErrorCode;

use super::{auth, images, labs, server, users};
use crate::api::request_parse::parse_value;
use crate::api::websocket::connection::Connection;
use crate::api::websocket::messages::{RpcError, ServerMessage};
use crate::auth::context::AuthContext;
use crate::auth::middleware;
use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::services::progress::ProgressSender;

const RPC_MSG_AUTH_REQUIRED: &str = "Authentication required";
const RPC_MSG_USER_ADMIN_ONLY_CREATE: &str = "Access denied: only administrators can create users";
const RPC_MSG_USER_ADMIN_ONLY_LIST: &str = "Access denied: only administrators can list users";
const RPC_MSG_USER_ADMIN_ONLY_DELETE: &str = "Access denied: only administrators can delete users";
const RPC_MSG_ADMIN_ONLY_CLEAN: &str =
    "Access denied: only administrators can run clean operations";
const RPC_MSG_ADMIN_ONLY_BACKUP: &str =
    "Access denied: only administrators can back up the database";
const RPC_MSG_ADMIN_ONLY_IMAGE_IMPORT: &str =
    "Access denied: only administrators can import images";
const RPC_MSG_ADMIN_ONLY_IMAGE_SCAN: &str = "Access denied: only administrators can scan images";
const RPC_MSG_ADMIN_ONLY_IMAGE_DELETE: &str =
    "Access denied: only administrators can delete images";
const RPC_MSG_ADMIN_ONLY_IMAGE_SET_DEFAULT: &str =
    "Access denied: only administrators can set the default image";
const RPC_MSG_SERIALIZE_FAILED: &str = "Failed to serialize response";
const RPC_MSG_ADMIN_ONLY_CONTAINER_PULL: &str =
    "Access denied: only administrators can pull container images";
const RPC_MSG_ADMIN_ONLY_IMAGE_DOWNLOAD: &str =
    "Access denied: only administrators can download images";

/// Every RPC method the server answers, who may call it, and whether it streams progress
/// messages before its response.
pub const METHODS: &[Method] = &[
    Method::new("auth.login", Access::Public, false),
    Method::new("auth.validate", Access::Public, false),
    Method::new("auth.logout", Access::User, false),
    Method::new("labs.list", Access::User, false),
    Method::new("inspect", Access::User, false),
    Method::new("download", Access::User, false),
    Method::new("down", Access::User, false),
    Method::new("cancel", Access::User, false),
    Method::new("node.secrets", Access::User, false),
    Method::new("node.host_keys", Access::User, false),
    Method::new("link.update_impairment", Access::User, false),
    Method::new("up", Access::User, true),
    Method::new("destroy", Access::User, true),
    Method::new("resume", Access::User, true),
    Method::new("redeploy", Access::User, true),
    Method::new("clean", Access::Admin(RPC_MSG_ADMIN_ONLY_CLEAN), false),
    Method::new(
        "server.backup",
        Access::Admin(RPC_MSG_ADMIN_ONLY_BACKUP),
        false,
    ),
    Method::new("image.list", Access::User, false),
    Method::new("image.show", Access::User, false),
    Method::new(
        "image.delete",
        Access::Admin(RPC_MSG_ADMIN_ONLY_IMAGE_DELETE),
        false,
    ),
    Method::new(
        "image.set_default",
        Access::Admin(RPC_MSG_ADMIN_ONLY_IMAGE_SET_DEFAULT),
        false,
    ),
    Method::new(
        "image.scan",
        Access::Admin(RPC_MSG_ADMIN_ONLY_IMAGE_SCAN),
        false,
    ),
    Method::new(
        "image.import",
        Access::Admin(RPC_MSG_ADMIN_ONLY_IMAGE_IMPORT),
        true,
    ),
    Method::new(
        "image.pull",
        Access::Admin(RPC_MSG_ADMIN_ONLY_CONTAINER_PULL),
        true,
    ),
    Method::new(
        "image.download",
        Access::Admin(RPC_MSG_ADMIN_ONLY_IMAGE_DOWNLOAD),
        true,
    ),
    Method::new(
        "user.create",
        Access::Admin(RPC_MSG_USER_ADMIN_ONLY_CREATE),
        false,
    ),
    Method::new(
        "user.list",
        Access::Admin(RPC_MSG_USER_ADMIN_ONLY_LIST),
        false,
    ),
    Method::new(
        "user.delete",
        Access::Admin(RPC_MSG_USER_ADMIN_ONLY_DELETE),
        false,
    ),
    Method::new("user.passwd", Access::User, false),
    Method::new("user.device_password.set", Access::User, false),
    Method::new("user.info", Access::User, false),
    Method::new("user.ssh_key.add", Access::User, false),
    Method::new("user.ssh_key.remove", Access::User, false),
];

/// An RPC method: its name, who may call it, and whether it streams.
#[derive(Debug, Clone, Copy)]
pub struct Method {
    pub name: &'static str,
    pub access: Access,
    pub streaming: bool,
}

/// Who may call an RPC method. Services still apply their own rules, such as lab ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Anyone; the method takes no token.
    Public,
    /// Any authenticated user.
    User,
    /// Admins only; others get the message.
    Admin(&'static str),
}

impl Method {
    const fn new(name: &'static str, access: Access, streaming: bool) -> Self {
        Self {
            name,
            access,
            streaming,
        }
    }
}

/// Answer one RPC request on `connection`: authenticate it as its method requires, run it,
/// forward its progress if it streams, and send its response.
pub async fn dispatch(
    id: String,
    method: String,
    params: Value,
    state: &AppState,
    connection: &Arc<Connection>,
) {
    let start = Instant::now();
    let result = call(&method, params, state, connection).await;
    let response = match result {
        Ok(result) => ServerMessage::RpcResponse {
            id,
            result: Some(result),
            error: None,
        },
        Err(error) => ServerMessage::RpcResponse {
            id,
            result: None,
            error: Some(error),
        },
    };
    send(connection, &response).await;
    state.metrics.rpc_duration.record(
        start.elapsed().as_secs_f64(),
        &[KeyValue::new("rpc.method", method)],
    );
}

async fn call(
    method: &str,
    params: Value,
    state: &AppState,
    connection: &Arc<Connection>,
) -> Result<Value, RpcError> {
    let Some(entry) = METHODS.iter().find(|entry| entry.name == method) else {
        return Err(RpcError {
            code: RpcErrorCode::MethodNotFound,
            message: format!("Method '{method}' not found"),
            context: None,
            data: None,
        });
    };
    let ctx = match entry.access {
        Access::Public => return public_call(method, params, state, connection).await,
        Access::User => authenticate(&params, state).await?,
        Access::Admin(denied) => require_admin(authenticate(&params, state).await?, denied)?,
    };
    if !entry.streaming {
        return user_call(method, &ctx, params, state).await;
    }
    let (progress, forwarding) = forward_progress(connection);
    let result = streaming_call(method, &ctx, params, state, progress).await;
    // The progress sender has been dropped, so forwarding ends once it has sent the rest.
    if let Err(error) = forwarding.await {
        tracing::warn!(%method, %error, "Progress forwarding task failed");
    }
    result
}

async fn public_call(
    method: &str,
    params: Value,
    state: &AppState,
    connection: &Connection,
) -> Result<Value, RpcError> {
    match method {
        "auth.login" => auth::login(params, state, connection.peer).await,
        "auth.validate" => auth::validate(params, state).await,
        _ => Err(unrouted(method)),
    }
}

async fn user_call(
    method: &str,
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
) -> Result<Value, RpcError> {
    match method {
        "auth.logout" => auth::logout(ctx, state).await,
        "labs.list" => labs::list(ctx, state).await,
        "inspect" => labs::inspect(ctx, params, state).await,
        "download" => labs::download(ctx, params, state).await,
        "down" => labs::down(ctx, params, state).await,
        "cancel" => labs::cancel(ctx, params, state).await,
        "node.secrets" => labs::node_secrets(ctx, params, state).await,
        "node.host_keys" => labs::node_host_keys(ctx, params, state).await,
        "link.update_impairment" => labs::update_impairment(ctx, params, state).await,
        "clean" => labs::clean(ctx, params, state).await,
        "server.backup" => server::backup(ctx, state).await,
        "image.list" => images::list(params, state).await,
        "image.show" => images::show(params, state).await,
        "image.delete" => images::delete(params, state).await,
        "image.set_default" => images::set_default(params, state).await,
        "image.scan" => images::scan(params, state).await,
        "user.create" => users::create(ctx, params, state).await,
        "user.list" => users::list(ctx, state).await,
        "user.delete" => users::delete(ctx, params, state).await,
        "user.passwd" => users::change_password(ctx, params, state).await,
        "user.device_password.set" => users::set_device_password(ctx, params, state).await,
        "user.info" => users::info(ctx, params, state).await,
        "user.ssh_key.add" => users::add_ssh_key(ctx, params, state).await,
        "user.ssh_key.remove" => users::remove_ssh_key(ctx, params, state).await,
        _ => Err(unrouted(method)),
    }
}

async fn streaming_call(
    method: &str,
    ctx: &AuthContext,
    params: Value,
    state: &AppState,
    progress: ProgressSender,
) -> Result<Value, RpcError> {
    match method {
        "up" => labs::up(ctx, params, state, progress).await,
        "destroy" => labs::destroy(ctx, params, state, progress).await,
        "resume" => labs::resume(ctx, params, state, progress).await,
        "redeploy" => labs::redeploy(ctx, params, state, progress).await,
        "image.import" => images::import(params, state, progress).await,
        "image.pull" => images::pull(params, state, progress).await,
        "image.download" => images::download(params, state, progress).await,
        _ => Err(unrouted(method)),
    }
}

/// A method in `METHODS` that no call table handles; a server bug.
fn unrouted(method: &str) -> RpcError {
    tracing::error!(%method, "RPC method is listed but not routed");
    RpcError {
        code: RpcErrorCode::InternalError,
        message: format!("Method '{method}' is not routed"),
        context: None,
        data: None,
    }
}

async fn authenticate(params: &Value, state: &AppState) -> Result<AuthContext, RpcError> {
    middleware::authenticate_request(params, state)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "RPC authentication failed");
            RpcError {
                code: RpcErrorCode::AuthRequired,
                message: RPC_MSG_AUTH_REQUIRED.to_string(),
                context: None,
                data: None,
            }
        })
}

fn require_admin(ctx: AuthContext, denied: &str) -> Result<AuthContext, RpcError> {
    if ctx.is_admin {
        return Ok(ctx);
    }
    tracing::warn!(username = %ctx.username, "Non-admin called an admin-only RPC method");
    Err(RpcError {
        code: RpcErrorCode::AccessDenied,
        message: denied.to_string(),
        context: None,
        data: None,
    })
}

/// A progress sender whose messages a task forwards to `connection` until the sender drops.
fn forward_progress(connection: &Arc<Connection>) -> (ProgressSender, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let connection = Arc::clone(connection);
    let forwarding = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if let Err(error) = connection.send(message).await {
                tracing::debug!(?error, "Client gone; dropping progress message");
            }
        }
    });
    (ProgressSender::new(tx), forwarding)
}

async fn send(connection: &Arc<Connection>, response: &ServerMessage) {
    match serde_json::to_string(response) {
        Ok(json) => {
            if let Err(error) = connection.send(Message::Text(json.into())).await {
                tracing::debug!(?error, "Client gone; dropping RPC response");
            }
        }
        Err(error) => tracing::error!(%error, "Failed to serialize RPC response"),
    }
}

/// Parse `params` into a request; `message` names what was expected. When they do not parse,
/// the context is a fixed message with the path of the field that is wrong, never the value
/// sent, which may be a password or a secret file's contents.
pub(super) fn parse<T: DeserializeOwned>(params: &Value, message: &str) -> Result<T, RpcError> {
    parse_value(params).map_err(|error| RpcError {
        code: RpcErrorCode::InvalidParams,
        message: message.to_string(),
        context: Some(error.message),
        data: None,
    })
}

/// The JSON result of a service call, or its error; `failure` describes a failure the service
/// did not classify.
pub(super) fn respond<T: Serialize>(
    result: anyhow::Result<T>,
    failure: &str,
) -> Result<Value, RpcError> {
    let response = result.map_err(|error| service_error(&error, failure))?;
    serde_json::to_value(response).map_err(|error| RpcError {
        code: RpcErrorCode::InternalError,
        message: RPC_MSG_SERIALIZE_FAILED.to_string(),
        context: Some(error.to_string()),
        data: None,
    })
}

/// Build the RPC error for a failed service call.
///
/// Typed service failures keep their own codes, so clients can tell "not allowed", "does not
/// exist", "bad request", "no room in the server's memory" and "CPU pins held" from operational
/// failures. Others carry the error's message chain, never its debug form.
pub(super) fn service_error(error: &anyhow::Error, failure: &str) -> RpcError {
    let (code, message, data) = match ServiceError::find(error) {
        Some(access @ ServiceError::Denied(_)) => {
            (RpcErrorCode::AccessDenied, access.to_string(), None)
        }
        Some(ServiceError::NotFound(message)) => (RpcErrorCode::NotFound, message.clone(), None),
        // JSON-RPC has no conflict code; the message tells the caller what to change.
        Some(ServiceError::Conflict(message)) => (RpcErrorCode::ServerError, message.clone(), None),
        Some(ServiceError::Invalid(message)) => {
            (RpcErrorCode::InvalidParams, message.clone(), None)
        }
        Some(ServiceError::InsufficientMemory { message, shortfall }) => (
            RpcErrorCode::InsufficientMemory,
            message.clone(),
            serde_json::to_value(shortfall).ok(),
        ),
        Some(ServiceError::CpuPinsHeld { message, conflict }) => (
            RpcErrorCode::CpuPinsHeld,
            message.clone(),
            serde_json::to_value(conflict).ok(),
        ),
        None => {
            tracing::warn!(error = ?error, "RPC call failed");
            return RpcError {
                code: RpcErrorCode::ServerError,
                message: failure.to_string(),
                context: Some(format!("{error:#}")),
                data: None,
            };
        }
    };
    RpcError {
        code,
        message,
        context: None,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::api_spec::{AuthRequirement, build_spec};
    use shared::error::{CpuPinConflict, CpuPinRole, MemoryShortfall};
    use std::collections::HashSet;

    /// Spec operations whose RPC method the server does not answer.
    const RPC_UNROUTED: &[&str] = &["image.upload"];
    /// Methods the server answers that the spec does not list yet.
    const NOT_IN_SPEC: &[&str] = &[
        "labs.list",
        "download",
        "node.secrets",
        "node.host_keys",
        "server.backup",
    ];

    #[test]
    fn api_documents_004_the_rpc_methods_answered_are_the_ones_the_spec_lists_each_once() {
        let mut names = HashSet::new();
        for method in METHODS {
            assert!(names.insert(method.name), "{} listed twice", method.name);
        }
        let spec = build_spec();
        let mut differences = Vec::new();
        for op in &spec.operations {
            let name = op.transports.rpc.method;
            let Some(method) = METHODS.iter().find(|method| method.name == name) else {
                if !RPC_UNROUTED.contains(&name) {
                    differences.push(format!("{name}: in the spec, not routed"));
                }
                continue;
            };
            let access_agrees = matches!(
                (op.auth, method.access),
                (AuthRequirement::None, Access::Public)
                    | (AuthRequirement::Authenticated, Access::User)
                    | (AuthRequirement::Admin, Access::Admin(_))
            );
            if !access_agrees {
                differences.push(format!(
                    "{name}: spec {:?}, server {:?}",
                    op.auth, method.access
                ));
            }
            if op.transports.rpc.streaming != method.streaming {
                differences.push(format!(
                    "{name}: spec streaming {}, server {}",
                    op.transports.rpc.streaming, method.streaming
                ));
            }
        }
        for method in METHODS {
            let in_spec = spec
                .operations
                .iter()
                .any(|op| op.transports.rpc.method == method.name);
            if !in_spec && !NOT_IN_SPEC.contains(&method.name) {
                differences.push(format!("{}: routed, not in the spec", method.name));
            }
        }
        assert!(differences.is_empty(), "{differences:#?}");
    }

    #[test]
    fn api_errors_009_rpc_params_that_do_not_parse_name_the_path_never_the_value() {
        const SENT: i64 = 271_828_182;
        let cases: [(&str, Result<(), RpcError>); 4] = [
            (
                "the request is not valid at password",
                parse::<shared::data::LoginRequest>(
                    &serde_json::json!({ "username": "dev", "password": SENT }),
                    "bad login",
                )
                .map(drop),
            ),
            (
                "the request is not valid at password",
                parse::<shared::data::CreateUserRequest>(
                    &serde_json::json!({
                        "username": "dev", "password": SENT, "is_admin": false, "token": "t",
                    }),
                    "bad create",
                )
                .map(drop),
            ),
            (
                "the request is not valid at new_password",
                parse::<shared::data::ChangePasswordRequest>(
                    &serde_json::json!({ "username": "dev", "new_password": SENT, "token": "t" }),
                    "bad passwd",
                )
                .map(drop),
            ),
            (
                "the request is not valid",
                parse::<shared::data::SetDevicePasswordRequest>(
                    &serde_json::json!({
                        "username": "dev", "login_password": SENT.to_string(), "token": "t",
                    }),
                    "bad device password",
                )
                .map(drop),
            ),
        ];
        for (expected, result) in cases {
            let error = result.expect_err("parsed");
            assert!(matches!(error.code, RpcErrorCode::InvalidParams));
            assert_eq!(error.context.as_deref(), Some(expected));
            assert!(
                !format!("{error:?}").contains(&SENT.to_string()),
                "{error:?}"
            );
        }

        let error = parse::<shared::data::NodeSecretsRequest>(
            &serde_json::json!({
                "lab_id": "abc12345", "node_name": "r1",
                "files": [{ "name": "token", "mode": SENT.to_string(), "content": "x" }],
            }),
            "bad secrets",
        )
        .expect_err("parsed");
        assert_eq!(
            error.context.as_deref(),
            Some("the request is not valid at files[0].mode")
        );

        let error = parse::<shared::data::NodeSecretsRequest>(
            &serde_json::json!({
                "lab_id": "abc12345", "node_name": "r1",
                "files": [{ "name": "token", "content": SENT.to_string() }],
            }),
            "bad secrets",
        )
        .expect_err("parsed");
        assert_eq!(
            error.context.as_deref(),
            Some("the request is not valid at files[0]")
        );

        let import = |extra: &str| {
            let mut params = serde_json::json!({
                "model": "cisco_ios", "version": "1", "src": "/tmp/i.qcow2",
            });
            params[extra] = serde_json::json!(SENT);
            parse::<shared::data::ImportRequest>(&params, "bad import").expect_err("parsed")
        };
        assert_eq!(
            import("ztp_password").context.as_deref(),
            Some("the request is not valid at ztp_password")
        );
        let error = import(&format!("{SENT} key"));
        assert_eq!(error.context.as_deref(), Some("the request is not valid"));
        assert!(
            !format!("{error:?}").contains(&SENT.to_string()),
            "{error:?}"
        );
    }

    #[test]
    fn provisioner_secrecy_004_a_request_with_secrets_that_does_not_parse_is_answered_without_them()
    {
        let params = serde_json::json!({
            "lab_id": "abc12345",
            "node_name": "r1",
            // serde quotes a string found where a number belongs.
            "files": [{ "name": "token", "mode": "s3cret-value", "content": "x" }],
        });
        let error = parse::<shared::data::NodeSecretsRequest>(&params, "bad").expect_err("parsed");
        assert!(!format!("{error:?}").contains("s3cret-value"), "{error:?}");
    }

    #[test]
    fn api_errors_003_an_unknown_rpc_failure_is_a_server_error_with_its_chain_as_context() {
        let error = anyhow::anyhow!("disk full").context("Failed to write lab file");
        let rpc = service_error(&error, "Up operation failed");
        assert_eq!(rpc.code, RpcErrorCode::ServerError);
        assert_eq!(rpc.message, "Up operation failed");
        assert_eq!(
            rpc.context.as_deref(),
            Some("Failed to write lab file: disk full")
        );
        assert!(rpc.data.is_none());
    }

    #[test]
    fn api_errors_004_a_known_rpc_refusal_keeps_its_code_and_message_and_has_no_context() {
        let error = anyhow::Error::new(ServiceError::Invalid("bad lab ID".to_owned()));
        let rpc = service_error(&error, "Clean failed");
        assert_eq!(rpc.code, RpcErrorCode::InvalidParams);
        assert_eq!(rpc.message, "bad lab ID");
        assert!(rpc.context.is_none());
    }

    #[test]
    fn a_memory_refusal_has_its_own_code_and_the_shortfall_as_data() {
        let shortfall = MemoryShortfall {
            requested_mib: 16384,
            available_mib: 11614,
            numa_node: None,
        };
        let error = anyhow::Error::new(ServiceError::InsufficientMemory {
            message: "Not enough memory on the server".to_owned(),
            shortfall,
        })
        .context("Memory admission failed");
        let rpc = service_error(&error, "Up operation failed");
        assert_eq!(rpc.code, RpcErrorCode::InsufficientMemory);
        assert_eq!(rpc.message, "Not enough memory on the server");
        assert!(rpc.context.is_none());
        assert_eq!(
            rpc.data,
            Some(serde_json::json!({
                "requested_mib": 16384,
                "available_mib": 11614,
                "numa_node": null,
            }))
        );
    }

    #[test]
    fn a_pin_refusal_has_its_own_code_and_the_conflict_as_data() {
        let error = anyhow::Error::new(ServiceError::CpuPinsHeld {
            message: "Host CPU 2 is held".to_owned(),
            conflict: CpuPinConflict {
                host_cpus: vec![2, 3],
                requested_role: CpuPinRole::Vcpu,
                held_role: CpuPinRole::Vcpu,
            },
        })
        .context("CPU pinning validation failed");
        let rpc = service_error(&error, "Up operation failed");
        assert_eq!(rpc.code, RpcErrorCode::CpuPinsHeld);
        assert_eq!(rpc.code.code(), -32006);
        assert_eq!(rpc.message, "Host CPU 2 is held");
        assert!(rpc.context.is_none());
        assert_eq!(
            rpc.data,
            Some(serde_json::json!({
                "host_cpus": [2, 3],
                "requested_role": "vcpu",
                "held_role": "vcpu",
            }))
        );
    }
}
