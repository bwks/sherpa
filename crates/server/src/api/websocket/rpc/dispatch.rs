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
use shared::konst::{
    RPC_MSG_ADMIN_ONLY_BACKUP, RPC_MSG_ADMIN_ONLY_CLEAN, RPC_MSG_ADMIN_ONLY_CONTAINER_PULL,
    RPC_MSG_ADMIN_ONLY_IMAGE_DELETE, RPC_MSG_ADMIN_ONLY_IMAGE_DOWNLOAD,
    RPC_MSG_ADMIN_ONLY_IMAGE_IMPORT, RPC_MSG_ADMIN_ONLY_IMAGE_SCAN,
    RPC_MSG_ADMIN_ONLY_IMAGE_SET_DEFAULT, RPC_MSG_AUTH_REQUIRED, RPC_MSG_SERIALIZE_FAILED,
    RPC_MSG_USER_ADMIN_ONLY_CREATE, RPC_MSG_USER_ADMIN_ONLY_DELETE, RPC_MSG_USER_ADMIN_ONLY_LIST,
};

use super::{auth, images, labs, server, users};
use crate::api::websocket::connection::Connection;
use crate::api::websocket::messages::{RpcError, ServerMessage};
use crate::auth::context::AuthContext;
use crate::auth::middleware;
use crate::daemon::state::AppState;
use crate::services::error::ServiceError;
use crate::services::progress::ProgressSender;

/// Every RPC method the server answers, who may call it, and whether it streams progress
/// messages before its response.
pub const METHODS: &[Method] = &[
    Method::new("auth.login", Access::Public, false),
    Method::new("auth.validate", Access::Public, false),
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
        });
    };
    let ctx = match entry.access {
        Access::Public => return public_call(method, params, state).await,
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

async fn public_call(method: &str, params: Value, state: &AppState) -> Result<Value, RpcError> {
    match method {
        "auth.login" => auth::login(params, state).await,
        "auth.validate" => auth::validate(params, state),
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

/// Parse `params` into a request; `message` names what was expected.
pub(super) fn parse<T: DeserializeOwned>(params: Value, message: &str) -> Result<T, RpcError> {
    serde_json::from_value(params).map_err(|error| RpcError {
        code: RpcErrorCode::InvalidParams,
        message: message.to_string(),
        context: Some(error.to_string()),
    })
}

/// Parse `params` that may carry secret file contents. The parse error is not returned, as it
/// could quote them.
pub(super) fn parse_quietly<T: DeserializeOwned>(
    params: Value,
    message: &str,
) -> Result<T, RpcError> {
    serde_json::from_value(params).map_err(|_| RpcError {
        code: RpcErrorCode::InvalidParams,
        message: message.to_string(),
        context: None,
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
    })
}

/// Build the RPC error for a failed service call.
///
/// Typed service failures keep their own codes, so clients can tell "not allowed", "does not
/// exist" and "bad request" from operational failures. Others carry the error's message chain,
/// never its debug form.
pub(super) fn service_error(error: &anyhow::Error, failure: &str) -> RpcError {
    let (code, message) = match ServiceError::find(error) {
        Some(access @ ServiceError::Denied(_)) => (RpcErrorCode::AccessDenied, access.to_string()),
        Some(ServiceError::NotFound(message)) => (RpcErrorCode::NotFound, message.clone()),
        // JSON-RPC has no conflict code; the message tells the caller what to change.
        Some(ServiceError::Conflict(message)) => (RpcErrorCode::ServerError, message.clone()),
        Some(ServiceError::Invalid(message)) => (RpcErrorCode::InvalidParams, message.clone()),
        None => {
            tracing::warn!(error = ?error, "RPC call failed");
            return RpcError {
                code: RpcErrorCode::ServerError,
                message: failure.to_string(),
                context: Some(format!("{error:#}")),
            };
        }
    };
    RpcError {
        code,
        message,
        context: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::api_spec::{AuthRequirement, build_spec};
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
    fn method_names_are_unique() {
        let mut names = HashSet::new();
        for method in METHODS {
            assert!(names.insert(method.name), "{} listed twice", method.name);
        }
    }

    #[test]
    fn the_method_table_matches_the_published_spec() {
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
    fn requests_with_secrets_never_echo_them_when_they_do_not_parse() {
        let params = serde_json::json!({
            "lab_id": "abc12345",
            "node_name": "r1",
            // serde quotes a string found where a number belongs.
            "files": [{ "name": "token", "mode": "s3cret-value", "content": "x" }],
        });
        let loud =
            parse::<shared::data::NodeSecretsRequest>(params.clone(), "bad").expect_err("parsed");
        assert!(
            format!("{loud:?}").contains("s3cret-value"),
            "the test no longer shows a leak"
        );
        let quiet =
            parse_quietly::<shared::data::NodeSecretsRequest>(params, "bad").expect_err("parsed");
        assert!(quiet.context.is_none());
        assert!(!format!("{quiet:?}").contains("s3cret-value"));
    }

    #[test]
    fn unclassified_failures_send_the_message_chain_not_debug_output() {
        let error = anyhow::anyhow!("disk full").context("Failed to write lab file");
        let rpc = service_error(&error, "Up operation failed");
        assert_eq!(rpc.message, "Up operation failed");
        assert_eq!(
            rpc.context.as_deref(),
            Some("Failed to write lab file: disk full")
        );
    }

    #[test]
    fn typed_failures_keep_their_codes_and_send_no_context() {
        let error = anyhow::Error::new(ServiceError::Invalid("bad lab ID".to_owned()));
        let rpc = service_error(&error, "Clean failed");
        assert_eq!(rpc.code, RpcErrorCode::InvalidParams);
        assert_eq!(rpc.message, "bad lab ID");
        assert!(rpc.context.is_none());
    }
}
