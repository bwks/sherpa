use std::collections::BTreeMap;

use schemars::schema_for;
use serde::Serialize;
use serde_json::json;

use crate::data::{
    ChangePasswordRequest, ChangePasswordResponse, ContainerPullRequest, ContainerPullResponse,
    CreateUserRequest, CreateUserResponse, DeleteImageRequest, DeleteImageResponse,
    DeleteUserRequest, DeleteUserResponse, DestroyRequest, DestroyResponse, DownloadImageRequest,
    GetUserInfoRequest, GetUserInfoResponse, ImportRequest, ImportResponse, InspectRequest,
    InspectResponse, LabNodeActionResponse, ListImagesRequest, ListImagesResponse,
    ListUsersRequest, ListUsersResponse, LoginRequest, LoginResponse, RedeployRequest,
    RedeployResponse, ScanImagesRequest, ScanImagesResponse, SetDefaultImageRequest,
    SetDefaultImageResponse, ShowImageRequest, ShowImageResponse, UpRequest, UpResponse,
    ValidateRequest, ValidateResponse,
};

/// Top-level unified API specification
#[derive(Debug, Clone, Serialize)]
pub struct ApiSpec {
    /// Spec format version
    pub version: String,
    /// All available operations
    pub operations: Vec<OperationDef>,
    /// JSON Schema definitions for all request/response types
    pub schemas: BTreeMap<String, serde_json::Value>,
}

/// A single API operation with transport bindings
#[derive(Debug, Clone, Serialize)]
pub struct OperationDef {
    /// Operation name (e.g., "lab.create")
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Operation category
    pub category: Category,
    /// Authentication requirement
    pub auth: AuthRequirement,
    /// Whether this operation streams progress
    pub streaming: bool,
    /// JSON Schema reference for request type (key into schemas map)
    pub request_schema: Option<String>,
    /// JSON Schema reference for response type (key into schemas map)
    pub response_schema: Option<String>,
    /// Transport bindings
    pub transports: Transports,
}

/// Operation category
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Auth,
    Lab,
    Node,
    Image,
    User,
}

/// Authentication requirement level
#[derive(Debug, Clone, Serialize)]
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
    pub path: String,
    /// Path parameter names (e.g. `["id", "node"]`)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub path_params: Vec<String>,
    /// Streaming mechanism (null for non-streaming)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_type: Option<String>,
}

/// HTTP methods
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
}

/// WebSocket JSON-RPC binding
#[derive(Debug, Clone, Serialize)]
pub struct RpcBinding {
    /// JSON-RPC method name
    pub method: String,
}

/// CLI command binding
#[derive(Debug, Clone, Serialize)]
pub struct CliBinding {
    /// Full command path (e.g., "sherpa server image import")
    pub command: String,
}

/// Build the complete unified API specification
pub fn build_spec() -> ApiSpec {
    let operations = build_operations();
    let schemas = build_schemas();

    ApiSpec {
        version: "1.0.0".to_string(),
        operations,
        schemas,
    }
}

fn build_operations() -> Vec<OperationDef> {
    vec![
        // Auth operations
        OperationDef {
            name: "auth.login".to_string(),
            description: "Authenticate a user and receive a JWT token".to_string(),
            category: Category::Auth,
            auth: AuthRequirement::None,
            streaming: false,
            request_schema: Some("LoginRequest".to_string()),
            response_schema: Some("LoginResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/auth/login".to_string(),
                    path_params: vec![],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "auth.login".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa login".to_string(),
                },
            },
        },
        OperationDef {
            name: "auth.validate".to_string(),
            description: "Validate a JWT token and return user info".to_string(),
            category: Category::Auth,
            auth: AuthRequirement::None,
            streaming: false,
            request_schema: Some("ValidateRequest".to_string()),
            response_schema: Some("ValidateResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/auth/validate".to_string(),
                    path_params: vec![],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "auth.validate".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa whoami".to_string(),
                },
            },
        },
        // Lab operations
        OperationDef {
            name: "lab.create".to_string(),
            description: "Create and start a new lab from a manifest".to_string(),
            category: Category::Lab,
            auth: AuthRequirement::Authenticated,
            streaming: true,
            request_schema: Some("UpRequest".to_string()),
            response_schema: Some("UpResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/labs".to_string(),
                    path_params: vec![],
                    stream_type: Some("sse".to_string()),
                },
                rpc: RpcBinding {
                    method: "up".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa up".to_string(),
                },
            },
        },
        OperationDef {
            name: "lab.destroy".to_string(),
            description: "Destroy a lab and all its resources".to_string(),
            category: Category::Lab,
            auth: AuthRequirement::Authenticated,
            streaming: true,
            request_schema: Some("DestroyRequest".to_string()),
            response_schema: Some("DestroyResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Delete,
                    path: "/api/v1/labs/{id}".to_string(),
                    path_params: vec!["id".to_string()],
                    stream_type: Some("sse".to_string()),
                },
                rpc: RpcBinding {
                    method: "destroy".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa destroy".to_string(),
                },
            },
        },
        OperationDef {
            name: "lab.inspect".to_string(),
            description: "Get detailed information about a lab".to_string(),
            category: Category::Lab,
            auth: AuthRequirement::Authenticated,
            streaming: false,
            request_schema: Some("InspectRequest".to_string()),
            response_schema: Some("InspectResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Get,
                    path: "/api/v1/labs/{id}".to_string(),
                    path_params: vec!["id".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "inspect".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa inspect".to_string(),
                },
            },
        },
        OperationDef {
            name: "lab.down".to_string(),
            description: "Stop all or specific nodes in a lab".to_string(),
            category: Category::Lab,
            auth: AuthRequirement::Authenticated,
            streaming: false,
            request_schema: None,
            response_schema: Some("LabNodeActionResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/labs/{id}/down".to_string(),
                    path_params: vec!["id".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "down".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa down".to_string(),
                },
            },
        },
        OperationDef {
            name: "lab.resume".to_string(),
            description: "Resume all or specific stopped nodes in a lab".to_string(),
            category: Category::Lab,
            auth: AuthRequirement::Authenticated,
            streaming: false,
            request_schema: None,
            response_schema: Some("LabNodeActionResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/labs/{id}/resume".to_string(),
                    path_params: vec!["id".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "resume".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa resume".to_string(),
                },
            },
        },
        OperationDef {
            name: "lab.clean".to_string(),
            description: "Force-clean all resources for a lab without ownership check".to_string(),
            category: Category::Lab,
            auth: AuthRequirement::Admin,
            streaming: false,
            request_schema: None,
            response_schema: Some("DestroyResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/labs/{id}/clean".to_string(),
                    path_params: vec!["id".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "clean".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server clean".to_string(),
                },
            },
        },
        // Node operations
        OperationDef {
            name: "node.redeploy".to_string(),
            description: "Destroy and recreate a node with fresh configuration".to_string(),
            category: Category::Node,
            auth: AuthRequirement::Authenticated,
            streaming: true,
            request_schema: Some("RedeployRequest".to_string()),
            response_schema: Some("RedeployResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/labs/{id}/nodes/{node}/redeploy".to_string(),
                    path_params: vec!["id".to_string(), "node".to_string()],
                    stream_type: Some("sse".to_string()),
                },
                rpc: RpcBinding {
                    method: "redeploy".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa redeploy".to_string(),
                },
            },
        },
        // Image operations
        OperationDef {
            name: "image.list".to_string(),
            description: "List available images with optional filters".to_string(),
            category: Category::Image,
            auth: AuthRequirement::Authenticated,
            streaming: false,
            request_schema: Some("ListImagesRequest".to_string()),
            response_schema: Some("ListImagesResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Get,
                    path: "/api/v1/images".to_string(),
                    path_params: vec![],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "image.list".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa image list".to_string(),
                },
            },
        },
        OperationDef {
            name: "image.show".to_string(),
            description: "Show detailed information about an image".to_string(),
            category: Category::Image,
            auth: AuthRequirement::Authenticated,
            streaming: false,
            request_schema: Some("ShowImageRequest".to_string()),
            response_schema: Some("ShowImageResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Get,
                    path: "/api/v1/images/{model}".to_string(),
                    path_params: vec!["model".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "image.show".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa image show".to_string(),
                },
            },
        },
        OperationDef {
            name: "image.import".to_string(),
            description: "Import a disk image or container tar archive".to_string(),
            category: Category::Image,
            auth: AuthRequirement::Admin,
            streaming: false,
            request_schema: Some("ImportRequest".to_string()),
            response_schema: Some("ImportResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/images/import".to_string(),
                    path_params: vec![],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "image.import".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server image import".to_string(),
                },
            },
        },
        OperationDef {
            name: "image.delete".to_string(),
            description: "Delete an imported image".to_string(),
            category: Category::Image,
            auth: AuthRequirement::Admin,
            streaming: false,
            request_schema: Some("DeleteImageRequest".to_string()),
            response_schema: Some("DeleteImageResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Delete,
                    path: "/api/v1/images/{model}/{version}".to_string(),
                    path_params: vec!["model".to_string(), "version".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "image.delete".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server image delete".to_string(),
                },
            },
        },
        OperationDef {
            name: "image.set_default".to_string(),
            description: "Set the default version for an image model".to_string(),
            category: Category::Image,
            auth: AuthRequirement::Admin,
            streaming: false,
            request_schema: Some("SetDefaultImageRequest".to_string()),
            response_schema: Some("SetDefaultImageResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/images/{model}/{version}/default".to_string(),
                    path_params: vec!["model".to_string(), "version".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "image.set_default".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server image set-default".to_string(),
                },
            },
        },
        OperationDef {
            name: "image.scan".to_string(),
            description: "Scan filesystem and Docker for discoverable images".to_string(),
            category: Category::Image,
            auth: AuthRequirement::Admin,
            streaming: false,
            request_schema: Some("ScanImagesRequest".to_string()),
            response_schema: Some("ScanImagesResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/images/scan".to_string(),
                    path_params: vec![],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "image.scan".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server image scan".to_string(),
                },
            },
        },
        OperationDef {
            name: "image.pull".to_string(),
            description: "Pull a container image from an OCI registry".to_string(),
            category: Category::Image,
            auth: AuthRequirement::Admin,
            streaming: true,
            request_schema: Some("ContainerPullRequest".to_string()),
            response_schema: Some("ContainerPullResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/images/pull".to_string(),
                    path_params: vec![],
                    stream_type: Some("sse".to_string()),
                },
                rpc: RpcBinding {
                    method: "image.pull".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server image pull".to_string(),
                },
            },
        },
        OperationDef {
            name: "image.download".to_string(),
            description: "Download a VM image from a URL".to_string(),
            category: Category::Image,
            auth: AuthRequirement::Admin,
            streaming: true,
            request_schema: Some("DownloadImageRequest".to_string()),
            response_schema: None,
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/images/download".to_string(),
                    path_params: vec![],
                    stream_type: Some("sse".to_string()),
                },
                rpc: RpcBinding {
                    method: "image.download".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server image pull --url".to_string(),
                },
            },
        },
        // User operations
        OperationDef {
            name: "user.create".to_string(),
            description: "Create a new user account".to_string(),
            category: Category::User,
            auth: AuthRequirement::Admin,
            streaming: false,
            request_schema: Some("CreateUserRequest".to_string()),
            response_schema: Some("CreateUserResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/users".to_string(),
                    path_params: vec![],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "user.create".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server user create".to_string(),
                },
            },
        },
        OperationDef {
            name: "user.list".to_string(),
            description: "List all user accounts".to_string(),
            category: Category::User,
            auth: AuthRequirement::Admin,
            streaming: false,
            request_schema: Some("ListUsersRequest".to_string()),
            response_schema: Some("ListUsersResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Get,
                    path: "/api/v1/users".to_string(),
                    path_params: vec![],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "user.list".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server user list".to_string(),
                },
            },
        },
        OperationDef {
            name: "user.delete".to_string(),
            description: "Delete a user account".to_string(),
            category: Category::User,
            auth: AuthRequirement::Admin,
            streaming: false,
            request_schema: Some("DeleteUserRequest".to_string()),
            response_schema: Some("DeleteUserResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Delete,
                    path: "/api/v1/users/{username}".to_string(),
                    path_params: vec!["username".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "user.delete".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server user delete".to_string(),
                },
            },
        },
        OperationDef {
            name: "user.passwd".to_string(),
            description: "Change a user's password".to_string(),
            category: Category::User,
            auth: AuthRequirement::Authenticated,
            streaming: false,
            request_schema: Some("ChangePasswordRequest".to_string()),
            response_schema: Some("ChangePasswordResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Post,
                    path: "/api/v1/users/{username}/password".to_string(),
                    path_params: vec!["username".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "user.passwd".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server user passwd".to_string(),
                },
            },
        },
        OperationDef {
            name: "user.info".to_string(),
            description: "Get detailed information about a user".to_string(),
            category: Category::User,
            auth: AuthRequirement::Authenticated,
            streaming: false,
            request_schema: Some("GetUserInfoRequest".to_string()),
            response_schema: Some("GetUserInfoResponse".to_string()),
            transports: Transports {
                rest: RestBinding {
                    method: HttpMethod::Get,
                    path: "/api/v1/users/{username}".to_string(),
                    path_params: vec!["username".to_string()],
                    stream_type: None,
                },
                rpc: RpcBinding {
                    method: "user.info".to_string(),
                },
                cli: CliBinding {
                    command: "sherpa server user info".to_string(),
                },
            },
        },
    ]
}

fn add_schema<T: schemars::JsonSchema>(schemas: &mut BTreeMap<String, serde_json::Value>) {
    let schema = schema_for!(T);
    let name = T::schema_name().to_string();
    if let Ok(value) = serde_json::to_value(schema) {
        schemas.insert(name, value);
    }
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

    schemas
}

/// Recursively rewrite `$ref` paths from schemars format (`#/definitions/Foo`)
/// to OpenAPI 3.1 format (`#/components/schemas/Foo`).
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
        schemas.entry(name).or_insert(def);
    }
}

/// Build an OpenAPI 3.1.0 document from the unified API spec.
pub fn build_openapi() -> serde_json::Value {
    let spec = build_spec();

    let mut schemas: BTreeMap<String, serde_json::Value> = spec.schemas;
    extract_definitions(&mut schemas);
    for schema in schemas.values_mut() {
        rewrite_refs(schema);
    }

    // Build paths, grouping operations that share the same REST path
    let mut paths: BTreeMap<String, serde_json::Map<String, serde_json::Value>> = BTreeMap::new();

    for op in &spec.operations {
        let rest = &op.transports.rest;

        let method_key = match rest.method {
            HttpMethod::Get => "get",
            HttpMethod::Post => "post",
            HttpMethod::Delete => "delete",
        };

        let category_tag = match op.category {
            Category::Auth => "auth",
            Category::Lab => "lab",
            Category::Node => "node",
            Category::Image => "image",
            Category::User => "user",
        };

        // Security
        let security = match op.auth {
            AuthRequirement::None => json!([]),
            AuthRequirement::Authenticated | AuthRequirement::Admin => {
                json!([{"BearerAuth": []}, {"CookieAuth": []}])
            }
        };

        // Request body (skip for GET)
        let request_body = match rest.method {
            HttpMethod::Get => None,
            _ => op.request_schema.as_ref().map(|schema_name| {
                json!({
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": { "$ref": format!("#/components/schemas/{schema_name}") }
                        }
                    }
                })
            }),
        };

        // Response
        let success_response = if op.streaming {
            json!({
                "description": format!("{} (Server-Sent Events stream)", op.description),
                "content": {
                    "text/event-stream": {
                        "schema": { "type": "string" }
                    }
                }
            })
        } else if let Some(ref schema_name) = op.response_schema {
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
        responses.insert("200".to_string(), success_response);

        // Add error responses based on auth requirement
        responses.insert(
            "400".to_string(),
            json!({ "description": "Bad request" }),
        );
        responses.insert(
            "500".to_string(),
            json!({ "description": "Internal server error" }),
        );

        match op.auth {
            AuthRequirement::None => {}
            AuthRequirement::Authenticated => {
                responses.insert(
                    "401".to_string(),
                    json!({ "description": "Unauthorized" }),
                );
            }
            AuthRequirement::Admin => {
                responses.insert(
                    "401".to_string(),
                    json!({ "description": "Unauthorized" }),
                );
                responses.insert(
                    "403".to_string(),
                    json!({ "description": "Forbidden — admin privileges required" }),
                );
            }
        }

        // Build the operation object
        let mut operation = serde_json::Map::new();
        operation.insert("operationId".to_string(), json!(op.name));
        operation.insert("summary".to_string(), json!(op.description));
        operation.insert("tags".to_string(), json!([category_tag]));
        operation.insert("security".to_string(), security);
        operation.insert(
            "responses".to_string(),
            serde_json::Value::Object(responses),
        );

        if let Some(body) = request_body {
            operation.insert("requestBody".to_string(), body);
        }

        if matches!(op.auth, AuthRequirement::Admin) {
            operation.insert("x-admin-required".to_string(), json!(true));
        }

        // Path parameters at path item level (shared across methods)
        let path_entry = paths.entry(rest.path.clone()).or_default();

        if !rest.path_params.is_empty() && !path_entry.contains_key("parameters") {
            let params: Vec<serde_json::Value> = rest.path_params
                .iter()
                .map(|name| {
                    json!({
                        "name": name,
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    })
                })
                .collect();
            path_entry.insert("parameters".to_string(), json!(params));
        }

        path_entry.insert(
            method_key.to_string(),
            serde_json::Value::Object(operation),
        );
    }

    // Convert paths to Value
    let paths_value: serde_json::Map<String, serde_json::Value> = paths
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::Object(v)))
        .collect();

    // Convert schemas to Value
    let schemas_value: serde_json::Map<String, serde_json::Value> =
        schemas.into_iter().collect();

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Sherpa API",
            "description": "Sherpa lab management API — virtual machines, containers, and unikernels",
            "version": spec.version
        },
        "servers": [{ "url": "/" }],
        "paths": serde_json::Value::Object(paths_value),
        "components": {
            "schemas": serde_json::Value::Object(schemas_value),
            "securitySchemes": {
                "BearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT"
                },
                "CookieAuth": {
                    "type": "apiKey",
                    "in": "cookie",
                    "name": "sherpa_token"
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_spec_has_22_operations() {
        let spec = build_spec();
        assert_eq!(spec.operations.len(), 22);
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
        let mut names: Vec<&str> = spec.operations.iter().map(|op| op.name.as_str()).collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "Duplicate operation names found");
    }

    #[test]
    fn test_all_rpc_methods_covered() {
        let spec = build_spec();
        let rpc_methods: Vec<&str> = spec
            .operations
            .iter()
            .map(|op| op.transports.rpc.method.as_str())
            .collect();

        // These are the 22 RPC methods from the WebSocket handler
        let expected = vec![
            "auth.login",
            "auth.validate",
            "up",
            "destroy",
            "inspect",
            "down",
            "resume",
            "clean",
            "redeploy",
            "image.list",
            "image.show",
            "image.import",
            "image.delete",
            "image.set_default",
            "image.scan",
            "image.pull",
            "image.download",
            "user.create",
            "user.list",
            "user.delete",
            "user.passwd",
            "user.info",
        ];

        for method in &expected {
            assert!(
                rpc_methods.contains(method),
                "RPC method '{}' not found in spec",
                method
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
            if let Some(ref schema_name) = op.request_schema {
                assert!(
                    spec.schemas.contains_key(schema_name),
                    "Operation '{}' references request schema '{}' which is not in schemas map",
                    op.name,
                    schema_name
                );
            }
            if let Some(ref schema_name) = op.response_schema {
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
            .map(|op| op.transports.rest.path.as_str())
            .collect();
        expected_paths.sort();
        expected_paths.dedup();

        for path in &expected_paths {
            assert!(
                paths.contains_key(*path),
                "Path '{}' missing from OpenAPI document",
                path
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
