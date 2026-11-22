//! Reading a request's JSON body, form, query string or path parameters into its type. When
//! one does not parse, the reply and the log get a fixed message: the field that is wrong and
//! what is wrong with it, never the value sent. serde's own messages quote what was sent, a
//! password among them, so none of their text past a field name is kept.

use axum::extract::path::ErrorKind;
use axum::extract::rejection::{FormRejection, JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, RawPathParams, Request};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::errors::ApiError;

const JSON_DATA_PREFIX: &str = "Failed to deserialize the JSON body into the target type: ";
const FORM_PREFIX: &str = "Failed to deserialize form: ";
const FORM_BODY_PREFIX: &str = "Failed to deserialize form body: ";
const QUERY_PREFIX: &str = "Failed to deserialize query string: ";
const MSG_NOT_JSON: &str = "the body is not valid JSON";
const MSG_NOT_JSON_CONTENT_TYPE: &str =
    "the body must be sent with `Content-Type: application/json`";
const MSG_NOT_FORM_CONTENT_TYPE: &str = "the form must be sent with \
     `Content-Type: application/x-www-form-urlencoded`";
const MSG_UNREADABLE: &str = "the request could not be read";
const MSG_PATH: &str = "the path does not name what this route takes";
/// The messages serde gives for a field that is missing, not taken or given twice, which name
/// the field and nothing that was sent with it.
const MISSING_FIELD: &str = "missing field `";
const UNKNOWN_FIELD: &str = "unknown field `";
const DUPLICATE_FIELD: &str = "duplicate field `";

/// A request that did not parse, with the fixed message it is answered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvalidRequest {
    pub(crate) message: String,
}

/// A JSON request body, read as `T`.
#[derive(Debug)]
pub(crate) struct JsonBody<T>(pub T);

/// A `application/x-www-form-urlencoded` request body, or a GET request's query, read as `T`.
#[derive(Debug)]
pub(crate) struct FormBody<T>(pub T);

/// A request's query string, read as `T`.
#[derive(Debug)]
pub(crate) struct QueryParams<T>(pub T);

/// A request's path parameters, read as `T`.
#[derive(Debug)]
pub(crate) struct PathParams<T>(pub T);

impl InvalidRequest {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for InvalidRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InvalidRequest {}

impl IntoResponse for InvalidRequest {
    fn into_response(self) -> Response {
        tracing::debug!(reason = %self.message, "Refused a request that does not parse");
        ApiError::bad_request(self.message).into_response()
    }
}

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for JsonBody<T> {
    type Rejection = InvalidRequest;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        axum::Json::<T>::from_request(request, state)
            .await
            .map(|axum::Json(value)| Self(value))
            .map_err(|rejection| json_rejection(&rejection))
    }
}

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for FormBody<T> {
    type Rejection = InvalidRequest;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        axum::Form::<T>::from_request(request, state)
            .await
            .map(|axum::Form(value)| Self(value))
            .map_err(|rejection| form_rejection(&rejection))
    }
}

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for QueryParams<T> {
    type Rejection = InvalidRequest;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| Self(value))
            .map_err(|rejection| query_rejection(&rejection))
    }
}

impl<T: DeserializeOwned + Send, S: Send + Sync> FromRequestParts<S> for PathParams<T> {
    type Rejection = InvalidRequest;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(Self(value)),
            Err(rejection) => {
                let names: Vec<String> = RawPathParams::from_request_parts(parts, state)
                    .await
                    .map_or_else(
                        |_| Vec::new(),
                        |params| params.iter().map(|(name, _)| name.to_string()).collect(),
                    );
                Err(path_rejection(&rejection, &names))
            }
        }
    }
}

/// Read the JSON `bytes` as `T`, for a request that comes as a WebSocket message or as RPC
/// parameters rather than an HTTP body.
///
/// # Errors
///
/// Fails, with the fixed message of [`InvalidRequest`], when `bytes` are not JSON or not a `T`.
pub(crate) fn parse_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, InvalidRequest> {
    axum::Json::<T>::from_bytes(bytes)
        .map(|axum::Json(value)| value)
        .map_err(|rejection| json_rejection(&rejection))
}

/// Read `value`, a part of a request already read as JSON, as `T`.
///
/// # Errors
///
/// Fails, with the fixed message of [`InvalidRequest`], when `value` is not a `T`.
pub(crate) fn parse_value<T: DeserializeOwned>(value: &Value) -> Result<T, InvalidRequest> {
    // A `Value` always serializes; were it not to, the empty body is refused as not JSON.
    parse_json(&serde_json::to_vec(value).unwrap_or_default())
}

/// The fixed message of a JSON body that was refused.
fn json_rejection(rejection: &JsonRejection) -> InvalidRequest {
    match rejection {
        JsonRejection::JsonDataError(_) => data_error(&rejection.body_text(), JSON_DATA_PREFIX),
        JsonRejection::MissingJsonContentType(_) => InvalidRequest::new(MSG_NOT_JSON_CONTENT_TYPE),
        JsonRejection::BytesRejection(_) => InvalidRequest::new(MSG_UNREADABLE),
        _ => InvalidRequest::new(MSG_NOT_JSON),
    }
}

/// The fixed message of a form that was refused.
fn form_rejection(rejection: &FormRejection) -> InvalidRequest {
    match rejection {
        FormRejection::FailedToDeserializeForm(_) => {
            data_error(&rejection.body_text(), FORM_PREFIX)
        }
        FormRejection::FailedToDeserializeFormBody(_) => {
            data_error(&rejection.body_text(), FORM_BODY_PREFIX)
        }
        FormRejection::InvalidFormContentType(_) => InvalidRequest::new(MSG_NOT_FORM_CONTENT_TYPE),
        _ => InvalidRequest::new(MSG_UNREADABLE),
    }
}

/// The fixed message of a query string that was refused.
fn query_rejection(rejection: &QueryRejection) -> InvalidRequest {
    match rejection {
        QueryRejection::FailedToDeserializeQueryString(_) => {
            data_error(&rejection.body_text(), QUERY_PREFIX)
        }
        _ => InvalidRequest::new(MSG_UNREADABLE),
    }
}

/// The fixed message of path parameters that were refused, naming the parameter when axum
/// knows it, by its name or by its place among the route's parameter `names`.
fn path_rejection(rejection: &PathRejection, names: &[String]) -> InvalidRequest {
    let PathRejection::FailedToDeserializePathParams(error) = rejection else {
        return InvalidRequest::new(MSG_PATH);
    };
    let key = match error.kind() {
        ErrorKind::ParseErrorAtKey { key, .. }
        | ErrorKind::InvalidUtf8InPathParam { key }
        | ErrorKind::DeserializeError { key, .. } => Some(key.as_str()),
        ErrorKind::ParseErrorAtIndex { index, .. } => names.get(*index).map(String::as_str),
        _ => None,
    };
    match key.filter(|key| is_field_path(key)) {
        Some(key) => InvalidRequest::new(format!(
            "path parameter `{key}` has the wrong type or value"
        )),
        None => InvalidRequest::new(MSG_PATH),
    }
}

/// The fixed message of a body or query that is well formed but not the request's type, from
/// `text`, axum's message: `prefix`, then `<path>: <serde message>`, or the serde message alone
/// when the fault is in the request as a whole. Only the path, and the field name of a missing,
/// unknown or repeated field, are kept: the rest of serde's message can quote what was sent.
fn data_error(text: &str, prefix: &str) -> InvalidRequest {
    let detail = text.strip_prefix(prefix).unwrap_or_default();
    let (path, serde_message) = split_field_path(detail);
    let named = |problem: &str| -> Option<String> {
        let name = serde_message.strip_prefix(problem)?.split('`').next()?;
        is_field_path(name).then(|| field_of(path, name))
    };
    let message = if let Some(field) = named(MISSING_FIELD) {
        format!("field `{field}` is missing")
    } else if let Some(field) = named(UNKNOWN_FIELD) {
        format!("field `{field}` is not a field this request takes")
    } else if let Some(field) = named(DUPLICATE_FIELD) {
        format!("field `{field}` is given more than once")
    } else if let Some(path) = path {
        format!("field `{path}` has the wrong type or value")
    } else {
        "the request has the wrong type or value".to_string()
    };
    InvalidRequest::new(message)
}

/// The path of field `name` serde names in its message, at `path`, where it failed: the field
/// itself for one it does not take or got twice, and the object it is missing from otherwise.
fn field_of(path: Option<&str>, name: &str) -> String {
    match path {
        Some(path) if path == name || path.ends_with(&format!(".{name}")) => path.to_string(),
        Some(path) => format!("{path}.{name}"),
        None => name.to_string(),
    }
}

/// `detail` split into the path of the field serde failed at and serde's message about it.
/// The path comes first, ended by `: `, and only names fields and list positions; a message
/// with no such path is about the request as a whole.
fn split_field_path(detail: &str) -> (Option<&str>, &str) {
    match detail.split_once(": ") {
        Some((path, message)) if is_field_path(path) => (Some(path), message),
        _ => (None, detail),
    }
}

/// Whether `path` is a field path as serde gives one, `files[0].mode`: field names of
/// letters, digits, `_` and `-`, joined by `.`, each with any list positions after it.
fn is_field_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('.').all(|segment| {
            let (name, mut positions) =
                segment.split_at(segment.find('[').unwrap_or(segment.len()));
            let name_ok = name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !name_ok || (name.is_empty() && positions.is_empty()) {
                return false;
            }
            while let Some(rest) = positions.strip_prefix('[') {
                let Some((index, after)) = rest.split_once(']') else {
                    return false;
                };
                if index.is_empty() || !index.chars().all(|c| c.is_ascii_digit()) {
                    return false;
                }
                positions = after;
            }
            positions.is_empty()
        })
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, StatusCode, header};
    use axum::routing::{get, post};
    use http_body_util::BodyExt;
    use serde::Deserialize;
    use shared::data::{CreateUserRequest, ListImagesRequest, LoginRequest};
    use tokio::net::TcpListener;

    use super::*;
    use crate::api::websocket::messages::ClientMessage;
    use crate::log_capture::capture_log;

    /// A value a client sent where it does not belong, which must never come back.
    const SENT: &str = "hunter2-s3cret";

    #[derive(Debug, Deserialize)]
    #[expect(dead_code, reason = "only read to see whether a request parses")]
    struct Lookup {
        name: String,
        port: u16,
    }

    fn json_request(body: &str, content_type: Option<&str>) -> Request {
        let mut builder = Request::builder().method(Method::POST).uri("/");
        if let Some(content_type) = content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        builder.body(Body::from(body.to_string())).unwrap()
    }

    /// The status and the `error` object of `response`.
    async fn error_of(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, json["error"].clone())
    }

    /// A server on a loopback port with a route for each way a request is read, and its URL.
    async fn serve() -> String {
        let router = Router::new()
            .route(
                "/json",
                post(|JsonBody(_): JsonBody<LoginRequest>| async { "ok" }),
            )
            .route(
                "/form",
                post(|FormBody(_): FormBody<Lookup>| async { "ok" }),
            )
            .route(
                "/query",
                get(|QueryParams(_): QueryParams<ListImagesRequest>| async { "ok" }),
            )
            .route(
                "/path/{name}/{port}",
                get(|PathParams(_): PathParams<(String, u16)>| async { "ok" }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }

    /// The status and error message the server at `url` answers `request` with.
    async fn refusal_of(request: reqwest::RequestBuilder) -> (StatusCode, String) {
        let response = request.send().await.unwrap();
        let status = StatusCode::from_u16(response.status().as_u16()).unwrap();
        let body = response.text().await.unwrap();
        assert!(!body.contains(SENT), "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["error"]["code"], "BAD_REQUEST", "{body}");
        (
            status,
            json["error"]["details"].as_str().unwrap().to_string(),
        )
    }

    #[tokio::test]
    async fn api_errors_010_a_rest_json_body_that_does_not_parse_names_the_field_never_its_value() {
        let cases = [
            (
                format!(r#"{{"username":"dev","password":{{"x":"{SENT}"}}}}"#),
                "field `password` has the wrong type or value",
            ),
            (
                r#"{"username":"dev"}"#.to_string(),
                "field `password` is missing",
            ),
            (
                format!(r#"{{"username":"dev","password":"{SENT}""#),
                MSG_NOT_JSON,
            ),
        ];
        for (body, expected) in cases {
            let rejection = JsonBody::<LoginRequest>::from_request(
                json_request(&body, Some("application/json")),
                &(),
            )
            .await
            .expect_err("parsed");
            let (status, error) = error_of(rejection.into_response()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(error["code"], "BAD_REQUEST");
            assert_eq!(error["message"], "Invalid request");
            assert_eq!(error["details"], expected, "{body}");
            assert!(!error.to_string().contains(SENT));
        }

        let body =
            format!(r#"{{"username":"dev","password":"p","is_admin":"{SENT}","token":"t"}}"#);
        let rejection = JsonBody::<CreateUserRequest>::from_request(
            json_request(&body, Some("application/json")),
            &(),
        )
        .await
        .expect_err("parsed");
        assert_eq!(
            rejection.message,
            "field `is_admin` has the wrong type or value"
        );

        let body = format!(r#"{{"username":"dev","password":"{SENT}"}}"#);
        let rejection = JsonBody::<LoginRequest>::from_request(json_request(&body, None), &())
            .await
            .expect_err("parsed");
        assert_eq!(rejection.message, MSG_NOT_JSON_CONTENT_TYPE);
    }

    #[tokio::test]
    async fn api_errors_011_a_form_query_or_path_that_does_not_parse_names_the_field_never_its_value()
     {
        let url = serve().await;
        let client = reqwest::Client::new();

        let form = client
            .post(format!("{url}/form"))
            .header(
                header::CONTENT_TYPE.as_str(),
                "application/x-www-form-urlencoded",
            )
            .body(format!("name=r1&port={SENT}"));
        assert_eq!(
            refusal_of(form).await,
            (
                StatusCode::BAD_REQUEST,
                "field `port` has the wrong type or value".to_string()
            )
        );

        let query = client.get(format!("{url}/query?model={SENT}"));
        assert_eq!(
            refusal_of(query).await,
            (
                StatusCode::BAD_REQUEST,
                "field `model` has the wrong type or value".to_string()
            )
        );

        let path = client.get(format!("{url}/path/r1/{SENT}"));
        assert_eq!(
            refusal_of(path).await,
            (
                StatusCode::BAD_REQUEST,
                "path parameter `port` has the wrong type or value".to_string()
            )
        );

        let json = client
            .post(format!("{url}/json"))
            .header(header::CONTENT_TYPE.as_str(), "application/json")
            .body(format!(r#"{{"username":"dev","password":{SENT}}}"#));
        assert_eq!(
            refusal_of(json).await,
            (StatusCode::BAD_REQUEST, MSG_NOT_JSON.to_string())
        );
    }

    #[test]
    fn api_errors_013_a_request_that_does_not_parse_logs_nothing_of_what_was_sent() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ((), log) = capture_log(|| {
            runtime.block_on(async {
                let body = format!(r#"{{"username":"dev","password":["{SENT}"]}}"#);
                let rejection = JsonBody::<LoginRequest>::from_request(
                    json_request(&body, Some("application/json")),
                    &(),
                )
                .await
                .expect_err("parsed");
                drop(rejection.into_response());

                let form = Request::builder()
                    .method(Method::POST)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!("name=r1&port={SENT}")))
                    .unwrap();
                let rejection = FormBody::<Lookup>::from_request(form, &())
                    .await
                    .expect_err("parsed");
                drop(rejection.into_response());
            });
        });
        assert!(
            log.contains("Refused a request that does not parse"),
            "nothing was logged: {log}"
        );
        assert!(!log.contains(SENT), "{log}");
    }

    #[test]
    fn api_errors_012_a_websocket_message_that_is_not_a_request_is_refused_without_what_was_sent() {
        let cases = [
            (
                format!(r#"{{"type":"{SENT}","id":"1","method":"m","params":{{}}}}"#),
                "field `type` has the wrong type or value",
            ),
            (
                format!(r#"{{"type":"rpc_request","id":["{SENT}"],"method":"m","params":{{}}}}"#),
                "the request has the wrong type or value",
            ),
            (SENT.to_string(), MSG_NOT_JSON),
        ];
        for (text, expected) in cases {
            let error = parse_json::<ClientMessage>(text.as_bytes()).expect_err("parsed");
            assert_eq!(error.message, expected, "{text}");
        }
    }
}
