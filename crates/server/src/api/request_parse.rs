//! Reading a request's JSON body, form or query string into its type. A request that does not
//! parse is answered `400` with a fixed message and the path of the field that is wrong; never
//! serde's or axum's message, which can quote what was sent.

use std::future::ready;

use axum::body::Bytes;
use axum::extract::rejection::RawFormRejection;
use axum::extract::{FromRequest, FromRequestParts, RawForm, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use mime::Mime;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_path_to_error::Segment;

use super::errors::ApiError;

const MSG_INVALID: &str = "the request is not valid";
const MSG_NOT_JSON: &str = "the body is not valid JSON";
const MSG_NOT_JSON_CONTENT_TYPE: &str =
    "the body must be sent with `Content-Type: application/json`";
const MSG_NOT_FORM_CONTENT_TYPE: &str = "the form must be sent with \
     `Content-Type: application/x-www-form-urlencoded`";
const MSG_UNREADABLE: &str = "the request could not be read";

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
        if !is_json(request.headers()) {
            return Err(InvalidRequest::new(MSG_NOT_JSON_CONTENT_TYPE));
        }
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|_| InvalidRequest::new(MSG_UNREADABLE))?;
        parse_json(&bytes).map(Self)
    }
}

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for FormBody<T> {
    type Rejection = InvalidRequest;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let RawForm(bytes) = RawForm::from_request(request, state)
            .await
            .map_err(|rejection| match rejection {
                RawFormRejection::InvalidFormContentType(_) => {
                    InvalidRequest::new(MSG_NOT_FORM_CONTENT_TYPE)
                }
                _ => InvalidRequest::new(MSG_UNREADABLE),
            })?;
        parse_urlencoded(&bytes).map(Self)
    }
}

impl<T: DeserializeOwned + Send, S: Send + Sync> FromRequestParts<S> for QueryParams<T> {
    type Rejection = InvalidRequest;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let query = parts.uri.query().unwrap_or_default();
        ready(parse_urlencoded(query.as_bytes()).map(Self))
    }
}

/// Read the JSON `bytes` as `T`, for a request body or a WebSocket message.
///
/// # Errors
///
/// Fails, with the fixed message of [`InvalidRequest`], when `bytes` are not JSON or not a `T`.
pub(crate) fn parse_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, InvalidRequest> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let parsed = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        if error.inner().is_data() {
            invalid_at(error.path())
        } else {
            InvalidRequest::new(MSG_NOT_JSON)
        }
    })?;
    deserializer
        .end()
        .map_err(|_| InvalidRequest::new(MSG_NOT_JSON))?;
    Ok(parsed)
}

/// Read `value`, a part of a request already read as JSON, as `T`.
///
/// # Errors
///
/// Fails, with the fixed message of [`InvalidRequest`], when `value` is not a `T`.
pub(crate) fn parse_value<T: DeserializeOwned>(value: &Value) -> Result<T, InvalidRequest> {
    serde_path_to_error::deserialize(value).map_err(|error| invalid_at(error.path()))
}

/// Read the `application/x-www-form-urlencoded` `bytes`, a form body or a query string, as `T`.
fn parse_urlencoded<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, InvalidRequest> {
    serde_path_to_error::deserialize(serde_urlencoded::Deserializer::new(form_urlencoded::parse(
        bytes,
    )))
    .map_err(|error| invalid_at(error.path()))
}

/// Whether `headers` say the body is JSON: `application/json` or `application/*+json`.
fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Mime>().ok())
        .is_some_and(|content_type| {
            content_type.type_() == mime::APPLICATION
                && (content_type.subtype() == mime::JSON
                    || content_type.suffix() == Some(mime::JSON))
        })
}

/// The fixed message of a request that is not valid at `path`. The path is named only when
/// every segment is an index or a plain name, since a map key or an unknown field is what the
/// client sent.
fn invalid_at(path: &serde_path_to_error::Path) -> InvalidRequest {
    let plain = path.iter().all(|segment| match segment {
        Segment::Seq { .. } => true,
        Segment::Map { key } => is_name(key),
        Segment::Enum { variant } => is_name(variant),
        Segment::Unknown => false,
    });
    if plain && path.iter().next().is_some() {
        InvalidRequest::new(format!("{MSG_INVALID} at {path}"))
    } else {
        InvalidRequest::new(MSG_INVALID)
    }
}

/// Whether `name`, sent by a client, is a plain name of letters, digits, `_` and `-`, which
/// can be named back without quoting anything else.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, StatusCode};
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
    /// A password sent as a number, which must never come back either.
    const SENT_NUMBER: &str = "73104729561";

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

    /// The refusal of `body` posted as JSON and read as `T`.
    async fn json_refusal<T: DeserializeOwned + Debug>(body: &str) -> InvalidRequest {
        JsonBody::<T>::from_request(json_request(body, Some("application/json")), &())
            .await
            .expect_err("parsed")
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
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }

    /// The status and error message the server answers `request` with.
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
    async fn api_errors_010_a_rest_json_body_that_does_not_parse_names_the_path_never_the_value() {
        let cases = [
            (
                format!(r#"{{"username":"dev","password":{{"x":"{SENT}"}}}}"#),
                "the request is not valid at password",
            ),
            (
                format!(r#"{{"username":"dev","password":{SENT_NUMBER}}}"#),
                "the request is not valid at password",
            ),
            (r#"{"username":"dev"}"#.to_string(), MSG_INVALID),
            (
                format!(r#"{{"username":"dev","password":"{SENT}""#),
                MSG_NOT_JSON,
            ),
        ];
        for (body, expected) in cases {
            let rejection = json_refusal::<LoginRequest>(&body).await;
            let (status, error) = error_of(rejection.into_response()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(error["code"], "BAD_REQUEST");
            assert_eq!(error["message"], "Invalid request");
            assert_eq!(error["details"], expected, "{body}");
            assert!(!error.to_string().contains(SENT));
            assert!(!error.to_string().contains(SENT_NUMBER));
        }

        let body =
            format!(r#"{{"username":"dev","password":"p","is_admin":"{SENT}","token":"t"}}"#);
        let rejection = json_refusal::<CreateUserRequest>(&body).await;
        assert_eq!(rejection.message, "the request is not valid at is_admin");

        let body = format!(r#"{{"username":"dev","password":"{SENT}"}}"#);
        let rejection = JsonBody::<LoginRequest>::from_request(json_request(&body, None), &())
            .await
            .expect_err("parsed");
        assert_eq!(rejection.message, MSG_NOT_JSON_CONTENT_TYPE);
    }

    #[tokio::test]
    async fn api_errors_011_a_form_or_query_that_does_not_parse_names_the_path_never_the_value() {
        let url = serve().await;
        let client = reqwest::Client::new();
        let invalid_at = |path: &str| (StatusCode::BAD_REQUEST, format!("{MSG_INVALID} at {path}"));

        let form = |body: String| {
            client
                .post(format!("{url}/form"))
                .header(
                    header::CONTENT_TYPE.as_str(),
                    "application/x-www-form-urlencoded",
                )
                .body(body)
        };
        assert_eq!(
            refusal_of(form(format!("name=r1&port={SENT}"))).await,
            invalid_at("port")
        );
        assert_eq!(
            refusal_of(form(format!("name={SENT}"))).await,
            (StatusCode::BAD_REQUEST, MSG_INVALID.to_string())
        );

        let form = client
            .post(format!("{url}/form"))
            .header(header::CONTENT_TYPE.as_str(), "text/plain")
            .body(format!("name=r1&port={SENT}"));
        assert_eq!(
            refusal_of(form).await,
            (
                StatusCode::BAD_REQUEST,
                MSG_NOT_FORM_CONTENT_TYPE.to_string()
            )
        );

        let query = client.get(format!("{url}/query?model={SENT}"));
        assert_eq!(refusal_of(query).await, invalid_at("model"));

        let json = client
            .post(format!("{url}/json"))
            .header(header::CONTENT_TYPE.as_str(), "application/json")
            .body(format!(r#"{{"username":"dev","password":{SENT}}}"#));
        assert_eq!(
            refusal_of(json).await,
            (StatusCode::BAD_REQUEST, MSG_NOT_JSON.to_string())
        );
    }

    #[tokio::test]
    async fn api_errors_012_a_websocket_message_that_is_not_a_request_is_refused_without_what_was_sent()
     {
        let cases = [
            (
                format!(r#"{{"type":"{SENT}","id":"1","method":"m","params":{{}}}}"#),
                "the request is not valid at type",
            ),
            (
                format!(r#"{{"type":"rpc_request","id":["{SENT}"],"method":"m","params":{{}}}}"#),
                MSG_INVALID,
            ),
            (SENT.to_string(), MSG_NOT_JSON),
        ];
        for (text, expected) in cases {
            let error = parse_json::<ClientMessage>(text.as_bytes()).expect_err("parsed");
            assert_eq!(error.message, expected, "{text}");
        }
    }

    #[test]
    fn api_errors_013_a_request_that_does_not_parse_logs_nothing_of_what_was_sent() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ((), log) = capture_log(|| {
            runtime.block_on(async {
                for body in [
                    format!(r#"{{"username":"dev","password":{SENT_NUMBER}}}"#),
                    format!(r#"{{"username":"dev","password":{{"x":["{SENT}"]}}}}"#),
                ] {
                    drop(json_refusal::<LoginRequest>(&body).await.into_response());
                }

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
        assert!(!log.contains(SENT_NUMBER), "{log}");
    }
}
