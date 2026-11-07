use std::collections::BTreeMap;

use schemars::schema_for;
use serde::Serialize;
use serde_json::json;

use crate::data::{
    AddSshKeyRequest, CancelLabResponse, ChangePasswordRequest, ChangePasswordResponse,
    ContainerPullRequest, ContainerPullResponse, CreateUserRequest, CreateUserResponse,
    DeleteImageRequest, DeleteImageResponse, DeleteUserRequest, DeleteUserResponse, DestroyRequest,
    DestroyResponse, DownloadImageRequest, GetUserInfoRequest, GetUserInfoResponse, ImportRequest,
    ImportResponse, InspectRequest, InspectResponse, LabNodeActionResponse, ListImagesRequest,
    ListImagesResponse, ListUsersRequest, ListUsersResponse, LoginRequest, LoginResponse,
    RedeployRequest, RedeployResponse, RemoveSshKeyRequest, ScanImagesRequest, ScanImagesResponse,
    SetDefaultImageRequest, SetDefaultImageResponse, ShowImageRequest, ShowImageResponse,
    UpRequest, UpResponse, UpdateImpairmentRequest, UpdateImpairmentResponse, UserSshKeysResponse,
    ValidateRequest, ValidateResponse,
};

/// Top-level unified API specification
#[derive(Debug, Clone, Serialize)]
pub struct ApiSpec {
    /// Spec format version
    pub version: &'static str,
    /// All available operations
    pub operations: Vec<OperationDef>,
    /// JSON Schema definitions for all request/response types
    pub schemas: BTreeMap<String, serde_json::Value>,
}

/// A single API operation with transport bindings
#[derive(Debug, Clone, Serialize)]
pub struct OperationDef {
    /// Operation name (e.g., "lab.create")
    pub name: &'static str,
    /// Human-readable description
    pub description: &'static str,
    /// Operation category
    pub category: Category,
    /// Authentication requirement
    pub auth: AuthRequirement,
    /// Whether this operation streams progress
    pub streaming: bool,
    /// JSON Schema reference for request type (key into schemas map)
    pub request_schema: Option<&'static str>,
    /// JSON Schema reference for response type (key into schemas map)
    pub response_schema: Option<&'static str>,
    /// Transport bindings
    pub transports: Transports,
}

/// Operation category
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Admin,
    Auth,
    Lab,
    Link,
    Node,
    Image,
    User,
}

/// Authentication requirement level
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthRequirement {
    None,
    Authenticated,
    Admin,
}

/// Transport bindings for an operation
#[derive(Debug, Clone, Serialize)]
pub struct Transports {
    pub rest: RestBinding,
    pub rpc: RpcBinding,
    pub cli: CliBinding,
}

/// REST API binding
#[derive(Debug, Clone, Serialize)]
pub struct RestBinding {
    /// HTTP method
    pub method: HttpMethod,
    /// URL path pattern
    pub path: &'static str,
    /// Path parameter names (e.g. `["id", "node"]`)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub path_params: Vec<&'static str>,
    /// Streaming mechanism (null for non-streaming)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_type: Option<StreamType>,
}

/// HTTP methods
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
}

impl HttpMethod {
    /// The method's key in an `OpenAPI` path item.
    fn openapi_key(self) -> &'static str {
        match self {
            HttpMethod::Get => "get",
            HttpMethod::Post => "post",
            HttpMethod::Delete => "delete",
        }
    }
}

/// How a streaming operation sends its progress.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamType {
    /// Server-Sent Events
    Sse,
}

/// WebSocket JSON-RPC binding
#[derive(Debug, Clone, Serialize)]
pub struct RpcBinding {
    /// JSON-RPC method name
    pub method: &'static str,
}

/// CLI command binding
#[derive(Debug, Clone, Serialize)]
pub struct CliBinding {
    /// Full command path (e.g., "sherpa server image import")
    pub command: &'static str,
}

/// An operation as `OPERATIONS` declares it. Its published form, `OperationDef`, adds what follows
/// from it: the path parameters, from the `{name}` segments of the path, and whether it streams.
struct Operation {
    name: &'static str,
    description: &'static str,
    category: Category,
    auth: AuthRequirement,
    request_schema: Option<&'static str>,
    response_schema: Option<&'static str>,
    method: HttpMethod,
    path: &'static str,
    stream_type: Option<StreamType>,
    rpc_method: &'static str,
    cli_command: &'static str,
}

/// Every operation the API offers.
const OPERATIONS: &[Operation] = &[
    // Auth operations
    Operation {
        name: "auth.login",
        description: "Authenticate a user and receive a JWT token",
        category: Category::Auth,
        auth: AuthRequirement::None,
        request_schema: Some("LoginRequest"),
        response_schema: Some("LoginResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/auth/login",
        stream_type: None,
        rpc_method: "auth.login",
        cli_command: "sherpa login",
    },
    Operation {
        name: "auth.validate",
        description: "Validate a JWT token and return user info",
        category: Category::Auth,
        auth: AuthRequirement::None,
        request_schema: Some("ValidateRequest"),
        response_schema: Some("ValidateResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/auth/validate",
        stream_type: None,
        rpc_method: "auth.validate",
        cli_command: "sherpa whoami",
    },
    // Lab operations
    Operation {
        name: "lab.create",
        description: "Create and start a new lab from a manifest",
        category: Category::Lab,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("UpRequest"),
        response_schema: Some("UpResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/labs",
        stream_type: Some(StreamType::Sse),
        rpc_method: "up",
        cli_command: "sherpa up",
    },
    Operation {
        name: "lab.destroy",
        description: "Destroy a lab and all its resources",
        category: Category::Lab,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("DestroyRequest"),
        response_schema: Some("DestroyResponse"),
        method: HttpMethod::Delete,
        path: "/api/v1/labs/{id}",
        stream_type: Some(StreamType::Sse),
        rpc_method: "destroy",
        cli_command: "sherpa destroy",
    },
    Operation {
        name: "lab.inspect",
        description: "Get detailed information about a lab",
        category: Category::Lab,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("InspectRequest"),
        response_schema: Some("InspectResponse"),
        method: HttpMethod::Get,
        path: "/api/v1/labs/{id}",
        stream_type: None,
        rpc_method: "inspect",
        cli_command: "sherpa inspect",
    },
    Operation {
        name: "lab.down",
        description: "Stop all or specific nodes in a lab",
        category: Category::Lab,
        auth: AuthRequirement::Authenticated,
        request_schema: None,
        response_schema: Some("LabNodeActionResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/labs/{id}/down",
        stream_type: None,
        rpc_method: "down",
        cli_command: "sherpa down",
    },
    Operation {
        name: "lab.cancel",
        description: "Stop the operations running on a lab (up, redeploy, resume) at their next safe point and wait for them to end; a stopped up rolls back",
        category: Category::Lab,
        auth: AuthRequirement::Authenticated,
        request_schema: None,
        response_schema: Some("CancelLabResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/labs/{id}/cancel",
        stream_type: None,
        rpc_method: "cancel",
        cli_command: "Ctrl-C during sherpa up, redeploy or resume",
    },
    Operation {
        name: "lab.resume",
        description: "Resume all or specific stopped nodes in a lab and wait for booted nodes to become ready",
        category: Category::Lab,
        auth: AuthRequirement::Authenticated,
        request_schema: None,
        response_schema: Some("LabNodeActionResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/labs/{id}/resume",
        stream_type: None,
        rpc_method: "resume",
        cli_command: "sherpa resume",
    },
    Operation {
        name: "lab.clean",
        description: "Force-clean all resources for a lab without ownership check",
        category: Category::Admin,
        auth: AuthRequirement::Admin,
        request_schema: None,
        response_schema: Some("DestroyResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/admin/tools/labs/clean/{id}",
        stream_type: None,
        rpc_method: "clean",
        cli_command: "sherpa server clean",
    },
    // Node operations
    Operation {
        name: "node.redeploy",
        description: "Destroy and recreate a node with fresh configuration; only the current lab owner may redeploy",
        category: Category::Node,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("RedeployRequest"),
        response_schema: Some("RedeployResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/labs/{id}/nodes/{node}/redeploy",
        stream_type: Some(StreamType::Sse),
        rpc_method: "redeploy",
        cli_command: "sherpa redeploy",
    },
    // Image operations
    Operation {
        name: "image.list",
        description: "List available images with optional filters",
        category: Category::Image,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("ListImagesRequest"),
        response_schema: Some("ListImagesResponse"),
        method: HttpMethod::Get,
        path: "/api/v1/images",
        stream_type: None,
        rpc_method: "image.list",
        cli_command: "sherpa image list",
    },
    Operation {
        name: "image.show",
        description: "Show detailed information about an image",
        category: Category::Image,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("ShowImageRequest"),
        response_schema: Some("ShowImageResponse"),
        method: HttpMethod::Get,
        path: "/api/v1/images/{model}",
        stream_type: None,
        rpc_method: "image.show",
        cli_command: "sherpa image show",
    },
    Operation {
        name: "image.import",
        description: "Import a disk image or container tar archive",
        category: Category::Image,
        auth: AuthRequirement::Admin,
        request_schema: Some("ImportRequest"),
        response_schema: Some("ImportResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/images/import",
        stream_type: None,
        rpc_method: "image.import",
        cli_command: "sherpa server image import",
    },
    Operation {
        name: "image.delete",
        description: "Delete an imported image",
        category: Category::Image,
        auth: AuthRequirement::Admin,
        request_schema: Some("DeleteImageRequest"),
        response_schema: Some("DeleteImageResponse"),
        method: HttpMethod::Delete,
        path: "/api/v1/images/{model}/{version}",
        stream_type: None,
        rpc_method: "image.delete",
        cli_command: "sherpa server image delete",
    },
    Operation {
        name: "image.set_default",
        description: "Set the default version for an image model",
        category: Category::Image,
        auth: AuthRequirement::Admin,
        request_schema: Some("SetDefaultImageRequest"),
        response_schema: Some("SetDefaultImageResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/images/{model}/{version}/default",
        stream_type: None,
        rpc_method: "image.set_default",
        cli_command: "sherpa server image set-default",
    },
    Operation {
        name: "image.scan",
        description: "Scan filesystem and Docker for discoverable images",
        category: Category::Admin,
        auth: AuthRequirement::Admin,
        request_schema: Some("ScanImagesRequest"),
        response_schema: Some("ScanImagesResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/admin/tools/images/scan",
        stream_type: None,
        rpc_method: "image.scan",
        cli_command: "sherpa server image scan",
    },
    Operation {
        name: "image.pull",
        description: "Pull a container image from an OCI registry",
        category: Category::Image,
        auth: AuthRequirement::Admin,
        request_schema: Some("ContainerPullRequest"),
        response_schema: Some("ContainerPullResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/images/pull",
        stream_type: Some(StreamType::Sse),
        rpc_method: "image.pull",
        cli_command: "sherpa server image pull",
    },
    Operation {
        name: "image.download",
        description: "Download a VM image from a URL",
        category: Category::Image,
        auth: AuthRequirement::Admin,
        request_schema: Some("DownloadImageRequest"),
        response_schema: None,
        method: HttpMethod::Post,
        path: "/api/v1/images/download",
        stream_type: Some(StreamType::Sse),
        rpc_method: "image.download",
        cli_command: "sherpa server image pull --url",
    },
    Operation {
        name: "image.upload",
        description: "Upload an image file via multipart form data",
        category: Category::Image,
        auth: AuthRequirement::Admin,
        request_schema: None,
        response_schema: Some("ImportResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/images/upload",
        stream_type: None,
        rpc_method: "image.upload",
        cli_command: "sherpa server image upload",
    },
    // User operations
    Operation {
        name: "user.create",
        description: "Create a new user account",
        category: Category::Admin,
        auth: AuthRequirement::Admin,
        request_schema: Some("CreateUserRequest"),
        response_schema: Some("CreateUserResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/admin/users",
        stream_type: None,
        rpc_method: "user.create",
        cli_command: "sherpa server user create",
    },
    Operation {
        name: "user.list",
        description: "List all user accounts",
        category: Category::Admin,
        auth: AuthRequirement::Admin,
        request_schema: Some("ListUsersRequest"),
        response_schema: Some("ListUsersResponse"),
        method: HttpMethod::Get,
        path: "/api/v1/admin/users",
        stream_type: None,
        rpc_method: "user.list",
        cli_command: "sherpa server user list",
    },
    Operation {
        name: "user.delete",
        description: "Delete a user account",
        category: Category::Admin,
        auth: AuthRequirement::Admin,
        request_schema: Some("DeleteUserRequest"),
        response_schema: Some("DeleteUserResponse"),
        method: HttpMethod::Delete,
        path: "/api/v1/admin/users/{username}",
        stream_type: None,
        rpc_method: "user.delete",
        cli_command: "sherpa server user delete",
    },
    Operation {
        name: "user.passwd",
        description: "Change a user's password",
        category: Category::Admin,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("ChangePasswordRequest"),
        response_schema: Some("ChangePasswordResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/admin/users/{username}/password",
        stream_type: None,
        rpc_method: "user.passwd",
        cli_command: "sherpa server user passwd",
    },
    Operation {
        name: "user.info",
        description: "Get detailed information about a user",
        category: Category::Admin,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("GetUserInfoRequest"),
        response_schema: Some("GetUserInfoResponse"),
        method: HttpMethod::Get,
        path: "/api/v1/admin/users/{username}",
        stream_type: None,
        rpc_method: "user.info",
        cli_command: "sherpa server user info",
    },
    Operation {
        name: "user.ssh_key.add",
        description: "Add an SSH public key to a user: your own, or any user's for an admin",
        category: Category::Admin,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("AddSshKeyRequest"),
        response_schema: Some("UserSshKeysResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/admin/users/{username}/ssh-keys",
        stream_type: None,
        rpc_method: "user.ssh_key.add",
        cli_command: "sherpa server user ssh-key add",
    },
    Operation {
        name: "user.ssh_key.remove",
        description: "Remove an SSH public key, given as the key or its SHA256 fingerprint, from a user",
        category: Category::Admin,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("RemoveSshKeyRequest"),
        response_schema: Some("UserSshKeysResponse"),
        method: HttpMethod::Delete,
        path: "/api/v1/admin/users/{username}/ssh-keys",
        stream_type: None,
        rpc_method: "user.ssh_key.remove",
        cli_command: "sherpa server user ssh-key remove",
    },
    // Link operations
    Operation {
        name: "link.update_impairment",
        description: "Update link impairment (delay, jitter, loss, reorder, corruption) on a running P2p link, in one or both directions",
        category: Category::Link,
        auth: AuthRequirement::Authenticated,
        request_schema: Some("UpdateImpairmentRequest"),
        response_schema: Some("UpdateImpairmentResponse"),
        method: HttpMethod::Post,
        path: "/api/v1/labs/{lab_id}/links/{link_index}/impairment",
        stream_type: None,
        rpc_method: "link.update_impairment",
        cli_command: "sherpa link impair",
    },
];

impl From<&Operation> for OperationDef {
    fn from(operation: &Operation) -> Self {
        let path_params = operation
            .path
            .split('/')
            .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
            .collect();
        Self {
            name: operation.name,
            description: operation.description,
            category: operation.category,
            auth: operation.auth,
            streaming: operation.stream_type.is_some(),
            request_schema: operation.request_schema,
            response_schema: operation.response_schema,
            transports: Transports {
                rest: RestBinding {
                    method: operation.method,
                    path: operation.path,
                    path_params,
                    stream_type: operation.stream_type,
                },
                rpc: RpcBinding {
                    method: operation.rpc_method,
                },
                cli: CliBinding {
                    command: operation.cli_command,
                },
            },
        }
    }
}

/// Build the complete unified API specification
#[must_use]
pub fn build_spec() -> ApiSpec {
    ApiSpec {
        version: "1.0.0",
        operations: OPERATIONS.iter().map(OperationDef::from).collect(),
        schemas: build_schemas(),
    }
}

fn add_schema<T: schemars::JsonSchema>(schemas: &mut BTreeMap<String, serde_json::Value>) {
    schemas.insert(T::schema_name().to_string(), schema_for!(T).to_value());
}

fn build_schemas() -> BTreeMap<String, serde_json::Value> {
    let mut schemas = BTreeMap::new();

    // Auth
    add_schema::<LoginRequest>(&mut schemas);
    add_schema::<LoginResponse>(&mut schemas);
    add_schema::<ValidateRequest>(&mut schemas);
    add_schema::<ValidateResponse>(&mut schemas);

    // Lab lifecycle
    add_schema::<UpRequest>(&mut schemas);
    add_schema::<UpResponse>(&mut schemas);
    add_schema::<DestroyRequest>(&mut schemas);
    add_schema::<DestroyResponse>(&mut schemas);
    add_schema::<InspectRequest>(&mut schemas);
    add_schema::<InspectResponse>(&mut schemas);
    add_schema::<RedeployRequest>(&mut schemas);
    add_schema::<RedeployResponse>(&mut schemas);
    add_schema::<LabNodeActionResponse>(&mut schemas);
    add_schema::<CancelLabResponse>(&mut schemas);

    // Image management
    add_schema::<ListImagesRequest>(&mut schemas);
    add_schema::<ListImagesResponse>(&mut schemas);
    add_schema::<ShowImageRequest>(&mut schemas);
    add_schema::<ShowImageResponse>(&mut schemas);
    add_schema::<ImportRequest>(&mut schemas);
    add_schema::<ImportResponse>(&mut schemas);
    add_schema::<DeleteImageRequest>(&mut schemas);
    add_schema::<DeleteImageResponse>(&mut schemas);
    add_schema::<SetDefaultImageRequest>(&mut schemas);
    add_schema::<SetDefaultImageResponse>(&mut schemas);
    add_schema::<ScanImagesRequest>(&mut schemas);
    add_schema::<ScanImagesResponse>(&mut schemas);
    add_schema::<ContainerPullRequest>(&mut schemas);
    add_schema::<ContainerPullResponse>(&mut schemas);
    add_schema::<DownloadImageRequest>(&mut schemas);

    // Link operations
    add_schema::<UpdateImpairmentRequest>(&mut schemas);
    add_schema::<UpdateImpairmentResponse>(&mut schemas);

    // User management
    add_schema::<CreateUserRequest>(&mut schemas);
    add_schema::<CreateUserResponse>(&mut schemas);
    add_schema::<ListUsersRequest>(&mut schemas);
    add_schema::<ListUsersResponse>(&mut schemas);
    add_schema::<DeleteUserRequest>(&mut schemas);
    add_schema::<DeleteUserResponse>(&mut schemas);
    add_schema::<ChangePasswordRequest>(&mut schemas);
    add_schema::<ChangePasswordResponse>(&mut schemas);
    add_schema::<GetUserInfoRequest>(&mut schemas);
    add_schema::<GetUserInfoResponse>(&mut schemas);
    add_schema::<AddSshKeyRequest>(&mut schemas);
    add_schema::<RemoveSshKeyRequest>(&mut schemas);
    add_schema::<UserSshKeysResponse>(&mut schemas);

    schemas
}

/// Recursively rewrite `$ref` paths from schemars format (`#/definitions/Foo`)
/// to `OpenAPI` 3.1 format (`#/components/schemas/Foo`).
fn rewrite_refs(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(ref_str)) = map.get_mut("$ref")
                && let Some(name) = ref_str.strip_prefix("#/definitions/")
            {
                *ref_str = format!("#/components/schemas/{name}");
            }
            for v in map.values_mut() {
                rewrite_refs(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                rewrite_refs(v);
            }
        }
        _ => {}
    }
}

/// Extract nested `definitions` blocks from schemars-generated schemas
/// and hoist them into the top-level schemas map.
fn extract_definitions(schemas: &mut BTreeMap<String, serde_json::Value>) {
    let mut hoisted = BTreeMap::new();

    for schema in schemas.values_mut() {
        if let serde_json::Value::Object(map) = schema
            && let Some(serde_json::Value::Object(defs)) = map.remove("definitions")
        {
            for (name, def) in defs {
                hoisted.insert(name, def);
            }
        }
    }

    for (name, mut def) in hoisted {
        rewrite_refs(&mut def);
        match schemas.get(&name) {
            Some(existing) if *existing != def => {
                tracing::warn!(schema = %name, "Two types share a schema name; keeping the first");
            }
            Some(_) => {}
            None => {
                schemas.insert(name, def);
            }
        }
    }
}

/// The `OpenAPI` operation object for `op`: its tags, security, request body and responses.
fn openapi_operation(op: &OperationDef) -> serde_json::Value {
    let security = match op.auth {
        AuthRequirement::None => json!([]),
        AuthRequirement::Authenticated | AuthRequirement::Admin => {
            json!([{"BearerAuth": []}, {"CookieAuth": []}])
        }
    };

    let success_response = if op.streaming {
        json!({
            "description": format!("{} (Server-Sent Events stream)", op.description),
            "content": { "text/event-stream": { "schema": { "type": "string" } } }
        })
    } else if let Some(schema_name) = op.response_schema {
        json!({
            "description": op.description,
            "content": {
                "application/json": {
                    "schema": { "$ref": format!("#/components/schemas/{schema_name}") }
                }
            }
        })
    } else {
        json!({ "description": op.description })
    };

    let mut responses = serde_json::Map::new();
    responses.insert("200".to_owned(), success_response);
    responses.insert("400".to_owned(), json!({ "description": "Bad request" }));
    responses.insert(
        "500".to_owned(),
        json!({ "description": "Internal server error" }),
    );
    if matches!(
        op.auth,
        AuthRequirement::Authenticated | AuthRequirement::Admin
    ) {
        responses.insert("401".to_owned(), json!({ "description": "Unauthorized" }));
    }
    if matches!(op.auth, AuthRequirement::Admin) {
        responses.insert(
            "403".to_owned(),
            json!({ "description": "Forbidden — admin privileges required" }),
        );
    }

    let mut operation = serde_json::Map::new();
    operation.insert("operationId".to_owned(), json!(op.name));
    operation.insert("summary".to_owned(), json!(op.description));
    operation.insert("tags".to_owned(), json!([op.category]));
    operation.insert("security".to_owned(), security);
    operation.insert("responses".to_owned(), serde_json::Value::Object(responses));
    // A GET request has no body.
    if op.transports.rest.method != HttpMethod::Get
        && let Some(schema_name) = op.request_schema
    {
        operation.insert(
            "requestBody".to_owned(),
            json!({
                "required": true,
                "content": {
                    "application/json": {
                        "schema": { "$ref": format!("#/components/schemas/{schema_name}") }
                    }
                }
            }),
        );
    }
    if matches!(op.auth, AuthRequirement::Admin) {
        operation.insert("x-admin-required".to_owned(), json!(true));
    }
    serde_json::Value::Object(operation)
}

/// The `OpenAPI` path parameters for `names`, each a required string.
fn path_parameters(names: &[&str]) -> serde_json::Value {
    names
        .iter()
        .map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } }))
        .collect()
}

/// Build an `OpenAPI` 3.1.0 document from the unified API spec.
#[must_use]
pub fn build_openapi() -> serde_json::Value {
    let spec = build_spec();

    let mut schemas: BTreeMap<String, serde_json::Value> = spec.schemas;
    extract_definitions(&mut schemas);
    for schema in schemas.values_mut() {
        rewrite_refs(schema);
    }

    // Operations that share a REST path share its path item; the parameters are set once.
    let mut paths: BTreeMap<&str, serde_json::Map<String, serde_json::Value>> = BTreeMap::new();
    for op in &spec.operations {
        let rest = &op.transports.rest;
        let path_item = paths.entry(rest.path).or_default();
        if !rest.path_params.is_empty() && !path_item.contains_key("parameters") {
            path_item.insert("parameters".to_owned(), path_parameters(&rest.path_params));
        }
        path_item.insert(rest.method.openapi_key().to_owned(), openapi_operation(op));
    }

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Sherpa API",
            "description": "Sherpa lab management API — virtual machines, containers, and unikernels",
            "version": spec.version
        },
        "servers": [{ "url": "/" }],
        "paths": paths,
        "components": {
            "schemas": schemas,
            "securitySchemes": {
                "BearerAuth": { "type": "http", "scheme": "bearer", "bearerFormat": "JWT" },
                "CookieAuth": { "type": "apiKey", "in": "cookie", "name": "sherpa_token" }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_spec_has_27_operations() {
        let spec = build_spec();
        assert_eq!(spec.operations.len(), 27);
    }

    #[test]
    fn test_build_spec_has_schemas() {
        let spec = build_spec();
        assert!(!spec.schemas.is_empty());
        // Should have schemas for all referenced request/response types
        assert!(spec.schemas.contains_key("LoginRequest"));
        assert!(spec.schemas.contains_key("LoginResponse"));
        assert!(spec.schemas.contains_key("UpRequest"));
        assert!(spec.schemas.contains_key("DestroyResponse"));
        assert!(spec.schemas.contains_key("InspectResponse"));
        assert!(spec.schemas.contains_key("ListImagesResponse"));
        assert!(spec.schemas.contains_key("CreateUserRequest"));
    }

    #[test]
    fn test_build_spec_serializes_to_json() {
        let spec = build_spec();
        let json = serde_json::to_value(&spec).unwrap();
        assert!(json.is_object());
        assert!(json.get("version").is_some());
        assert!(json.get("operations").is_some());
        assert!(json.get("schemas").is_some());
    }

    #[test]
    fn test_all_operation_names_unique() {
        let spec = build_spec();
        let mut names: Vec<&str> = spec.operations.iter().map(|op| op.name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "Duplicate operation names found");
    }

    #[test]
    fn test_all_rpc_methods_covered() {
        let spec = build_spec();
        let rpc_methods: Vec<&str> = spec
            .operations
            .iter()
            .map(|op| op.transports.rpc.method)
            .collect();

        // These are the RPC methods from the WebSocket handler
        let expected = vec![
            "auth.login",
            "auth.validate",
            "up",
            "destroy",
            "inspect",
            "down",
            "cancel",
            "resume",
            "clean",
            "redeploy",
            "link.update_impairment",
            "image.list",
            "image.show",
            "image.import",
            "image.delete",
            "image.set_default",
            "image.scan",
            "image.pull",
            "image.download",
            "image.upload",
            "user.create",
            "user.list",
            "user.delete",
            "user.passwd",
            "user.info",
            "user.ssh_key.add",
            "user.ssh_key.remove",
        ];

        for method in &expected {
            assert!(
                rpc_methods.contains(method),
                "RPC method '{method}' not found in spec"
            );
        }
        assert_eq!(rpc_methods.len(), expected.len());
    }

    #[test]
    fn test_streaming_operations_have_stream_type() {
        let spec = build_spec();
        let streaming_ops: Vec<&OperationDef> =
            spec.operations.iter().filter(|op| op.streaming).collect();

        assert_eq!(streaming_ops.len(), 5, "Expected 5 streaming operations");

        for op in &streaming_ops {
            assert!(
                op.transports.rest.stream_type.is_some(),
                "Streaming operation '{}' missing stream_type in REST binding",
                op.name
            );
        }
    }

    #[test]
    fn test_schema_references_valid() {
        let spec = build_spec();
        for op in &spec.operations {
            if let Some(schema_name) = op.request_schema {
                assert!(
                    spec.schemas.contains_key(schema_name),
                    "Operation '{}' references request schema '{}' which is not in schemas map",
                    op.name,
                    schema_name
                );
            }
            if let Some(schema_name) = op.response_schema {
                assert!(
                    spec.schemas.contains_key(schema_name),
                    "Operation '{}' references response schema '{}' which is not in schemas map",
                    op.name,
                    schema_name
                );
            }
        }
    }

    #[test]
    fn test_build_openapi_valid_structure() {
        let doc = build_openapi();
        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["title"], "Sherpa API");
        assert_eq!(doc["info"]["version"], "1.0.0");
        assert!(doc["paths"].is_object());
        assert!(doc["components"]["schemas"].is_object());
        assert!(doc["components"]["securitySchemes"].is_object());
    }

    #[test]
    fn test_build_openapi_has_all_paths() {
        let spec = build_spec();
        let doc = build_openapi();
        let paths = doc["paths"].as_object().unwrap();

        let mut expected_paths: Vec<&str> = spec
            .operations
            .iter()
            .map(|op| op.transports.rest.path)
            .collect();
        expected_paths.sort_unstable();
        expected_paths.dedup();

        for path in &expected_paths {
            assert!(
                paths.contains_key(*path),
                "Path '{path}' missing from OpenAPI document"
            );
        }
    }

    #[test]
    fn test_build_openapi_schema_refs_rewritten() {
        let doc = build_openapi();
        let json_str = serde_json::to_string(&doc).unwrap();
        assert!(
            !json_str.contains("#/definitions/"),
            "Found unrewritten #/definitions/ ref in OpenAPI document"
        );
    }

    #[test]
    fn test_build_openapi_security_schemes() {
        let doc = build_openapi();
        let schemes = &doc["components"]["securitySchemes"];
        assert_eq!(schemes["BearerAuth"]["type"], "http");
        assert_eq!(schemes["BearerAuth"]["scheme"], "bearer");
        assert_eq!(schemes["BearerAuth"]["bearerFormat"], "JWT");
        assert_eq!(schemes["CookieAuth"]["type"], "apiKey");
        assert_eq!(schemes["CookieAuth"]["in"], "cookie");
    }

    #[test]
    fn test_build_openapi_public_endpoints_no_security() {
        let doc = build_openapi();
        let login_op = &doc["paths"]["/api/v1/auth/login"]["post"];
        assert_eq!(login_op["security"], json!([]));
    }

    #[test]
    fn test_build_openapi_streaming_ops_use_sse() {
        let doc = build_openapi();
        // lab.create is POST /api/v1/labs and is streaming
        let create_lab = &doc["paths"]["/api/v1/labs"]["post"];
        assert!(
            create_lab["responses"]["200"]["content"]["text/event-stream"].is_object(),
            "Streaming operation should have text/event-stream response"
        );
    }

    #[test]
    fn test_build_openapi_path_parameters_extracted() {
        let doc = build_openapi();
        let lab_path = &doc["paths"]["/api/v1/labs/{id}"];
        let params = lab_path["parameters"].as_array().unwrap();
        assert!(
            params.iter().any(|p| p["name"] == "id"),
            "Path /api/v1/labs/{{id}} should have 'id' parameter"
        );
    }

    #[test]
    fn test_build_openapi_admin_ops_marked() {
        let doc = build_openapi();
        // image.import is admin-only: POST /api/v1/images/import
        let import_op = &doc["paths"]["/api/v1/images/import"]["post"];
        assert_eq!(
            import_op["x-admin-required"], true,
            "Admin operation should have x-admin-required: true"
        );
    }
}
