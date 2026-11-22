//! Reading a request's JSON body, form, query string or path parameters into its type. When
//! one does not parse, the reply and the log get a fixed message: the field that is wrong and
//! the kind of fault, never the value sent. The request is deserialized through
//! `serde_path_to_error`, which gives the field's path as data, with an error type that keeps
//! the kind of fault serde reports and none of its message, which can quote what was sent.

mod deserializer;

use axum::extract::rejection::{FormRejection, JsonRejection};
use axum::extract::{FromRequest, FromRequestParts, RawPathParams, Request};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::Deserializer;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_path_to_error::{Path, Segment};

use self::deserializer::{Fault, JsonValue, PathValues, TextPairs};
use super::errors::ApiError;

const MSG_NOT_JSON: &str = "the body is not valid JSON";
const MSG_NOT_JSON_CONTENT_TYPE: &str =
    "the body must be sent with `Content-Type: application/json`";
const MSG_NOT_FORM_CONTENT_TYPE: &str = "the form must be sent with \
     `Content-Type: application/x-www-form-urlencoded`";
const MSG_UNREADABLE: &str = "the request could not be read";
const MSG_PATH: &str = "the path does not name what this route takes";
const MSG_WRONG_SHAPE: &str = "the request has the wrong type or value";
const MSG_UNKNOWN_FIELD: &str = "the request has a field it does not take";

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
        let axum::Json(value) = axum::Json::<Value>::from_request(request, state)
            .await
            .map_err(|rejection| json_rejection(&rejection))?;
        from_json(value).map(Self)
    }
}

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for FormBody<T> {
    type Rejection = InvalidRequest;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Form(pairs) = axum::Form::<Vec<(String, String)>>::from_request(request, state)
            .await
            .map_err(|rejection| form_rejection(&rejection))?;
        from_text_pairs(pairs).map(Self)
    }
}

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for QueryParams<T> {
    type Rejection = InvalidRequest;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Query(pairs) =
            axum::extract::Query::<Vec<(String, String)>>::from_request_parts(parts, state)
                .await
                .map_err(|_| InvalidRequest::new(MSG_UNREADABLE))?;
        from_text_pairs(pairs).map(Self)
    }
}

impl<T: DeserializeOwned + Send, S: Send + Sync> FromRequestParts<S> for PathParams<T> {
    type Rejection = InvalidRequest;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let params: Vec<(String, String)> = RawPathParams::from_request_parts(parts, state)
            .await
            .map_err(|_| InvalidRequest::new(MSG_PATH))?
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        let names: Vec<String> = params.iter().map(|(name, _)| name.clone()).collect();
        serde_path_to_error::deserialize(PathValues(params))
            .map(Self)
            .map_err(|error| path_fault(error.path(), &names))
    }
}

/// Read the JSON `bytes` as `T`, for a request that comes as a WebSocket message or as RPC
/// parameters rather than an HTTP body.
///
/// # Errors
///
/// Fails, with the fixed message of [`InvalidRequest`], when `bytes` are not JSON or not a `T`.
pub(crate) fn parse_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, InvalidRequest> {
    let value = serde_json::from_slice(bytes).map_err(|_| InvalidRequest::new(MSG_NOT_JSON))?;
    from_json(value)
}

/// Read `value`, a part of a request already read as JSON, as `T`.
///
/// # Errors
///
/// Fails, with the fixed message of [`InvalidRequest`], when `value` is not a `T`.
pub(crate) fn parse_value<T: DeserializeOwned>(value: &Value) -> Result<T, InvalidRequest> {
    from_json(value.clone())
}

/// Read the JSON `value` as `T`.
fn from_json<T: DeserializeOwned>(value: Value) -> Result<T, InvalidRequest> {
    deserialize(JsonValue(value))
}

/// Read the name and value `pairs` of a form or a query string as `T`.
fn from_text_pairs<T: DeserializeOwned>(pairs: Vec<(String, String)>) -> Result<T, InvalidRequest> {
    deserialize(TextPairs(pairs))
}

/// Read `T` from `deserializer`, answering a fault with its fixed message.
fn deserialize<'de, T, D>(deserializer: D) -> Result<T, InvalidRequest>
where
    T: DeserializeOwned,
    D: Deserializer<'de, Error = Fault>,
{
    serde_path_to_error::deserialize(deserializer)
        .map_err(|error| fault_message(error.path(), error.inner()))
}

/// The fixed message of a JSON body that was refused before it was read as its type.
fn json_rejection(rejection: &JsonRejection) -> InvalidRequest {
    match rejection {
        JsonRejection::MissingJsonContentType(_) => InvalidRequest::new(MSG_NOT_JSON_CONTENT_TYPE),
        JsonRejection::BytesRejection(_) => InvalidRequest::new(MSG_UNREADABLE),
        _ => InvalidRequest::new(MSG_NOT_JSON),
    }
}

/// The fixed message of a form that was refused before it was read as its type.
fn form_rejection(rejection: &FormRejection) -> InvalidRequest {
    match rejection {
        FormRejection::InvalidFormContentType(_) => InvalidRequest::new(MSG_NOT_FORM_CONTENT_TYPE),
        _ => InvalidRequest::new(MSG_UNREADABLE),
    }
}

/// The fixed message of `fault`, found at `path` in a body, a form or a query string.
fn fault_message(path: &Path, fault: &Fault) -> InvalidRequest {
    let at = field_path(path);
    let message = match fault {
        Fault::Missing(name) => format!("field `{}` is missing", field_in(at, name)),
        Fault::Duplicate(name) => format!("field `{}` is given more than once", field_in(at, name)),
        Fault::Unknown(name) if is_name(name) => format!(
            "field `{}` is not a field this request takes",
            field_in(at, name)
        ),
        Fault::Unknown(_) => MSG_UNKNOWN_FIELD.to_string(),
        Fault::Invalid => match at {
            Some(at) => format!("field `{at}` has the wrong type or value"),
            None => MSG_WRONG_SHAPE.to_string(),
        },
    };
    InvalidRequest::new(message)
}

/// The fixed message of path parameters that did not parse at `path`, naming the parameter by
/// its name or by its place among the route's parameter `names`, or the lone one.
fn path_fault(path: &Path, names: &[String]) -> InvalidRequest {
    let name = match path.iter().next() {
        Some(Segment::Seq { index }) => names.get(*index),
        Some(Segment::Map { key }) => names.iter().find(|name| *name == key),
        _ if names.len() == 1 => names.first(),
        _ => None,
    };
    match name.filter(|name| is_name(name)) {
        Some(name) => InvalidRequest::new(format!(
            "path parameter `{name}` has the wrong type or value"
        )),
        None => InvalidRequest::new(MSG_PATH),
    }
}

/// `path` written as a field path, `files[0].mode`, up to the first segment that is not a
/// field's name or a list position: a map's keys are what was sent, and are named only when
/// they are plain names. `None` when the path is the request as a whole.
fn field_path(path: &Path) -> Option<String> {
    let mut written = String::new();
    for segment in path {
        match segment {
            Segment::Seq { index } => {
                written.push('[');
                written.push_str(&index.to_string());
                written.push(']');
            }
            Segment::Map { key } if is_name(key) => {
                if !written.is_empty() {
                    written.push('.');
                }
                written.push_str(key);
            }
            Segment::Enum { .. } => {}
            Segment::Map { .. } | Segment::Unknown => break,
        }
    }
    (!written.is_empty()).then_some(written)
}

/// The path of field `name`, which serde reports at the path `at`: the path itself when it
/// already ends at the field, and the field inside it otherwise.
fn field_in(at: Option<String>, name: &str) -> String {
    match at {
        Some(at) if at == name || at.ends_with(&format!(".{name}")) => at,
        Some(at) => format!("{at}.{name}"),
        None => name.to_string(),
    }
}

/// Whether `name`, sent by a client, is a plain field name of letters, digits, `_` and `-`,
/// which can be named back without quoting anything else.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
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
                post(|FormBody(lookup): FormBody<Lookup>| async move {
                    format!("{}:{}", lookup.name, lookup.port)
                }),
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

        let form = client
            .post(format!("{url}/form"))
            .header(
                header::CONTENT_TYPE.as_str(),
                "application/x-www-form-urlencoded",
            )
            .body(format!("name={SENT}&name=r2&port=22"));
        assert_eq!(
            refusal_of(form).await,
            (
                StatusCode::BAD_REQUEST,
                "field `name` is given more than once".to_string()
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
