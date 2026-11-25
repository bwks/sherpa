use anyhow::{Context, Result};
use reqwest::cookie::Jar;
use reqwest::header;
use reqwest::redirect::Policy;
use reqwest::{Client, Response};
use std::net::SocketAddr;
use std::sync::Arc;

/// HTTP test client with cookie jar support
pub struct TestHttpClient {
    client: Client,
    base_url: String,
    token: Option<String>,
}

impl TestHttpClient {
    /// Create a new HTTP client pointing at the given address
    pub fn new(addr: SocketAddr) -> Self {
        let jar = Arc::new(Jar::default());
        let client = Client::builder()
            .cookie_provider(jar)
            .redirect(Policy::none())
            .build()
            .unwrap_or_default();

        Self {
            client,
            base_url: format!("http://{addr}"),
            token: None,
        }
    }

    /// Set the Bearer token for authenticated requests
    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    /// GET request
    pub async fn get(&self, path: &str) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.get(&url);
        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }
        req.send().await.context("HTTP GET failed")
    }

    /// GET request with an explicit Cookie header.
    #[tracing::instrument(level = "debug", skip(self, cookie_header), fields(path))]
    pub async fn get_with_cookie(&self, path: &str, cookie_header: &str) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        self.client
            .get(&url)
            .header(header::COOKIE, cookie_header)
            .send()
            .await
            .context("HTTP GET with cookie failed")
    }

    /// POST request with form data
    pub async fn post_form(&self, path: &str, form: &[(&str, &str)]) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.post(&url).form(form);
        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }
        req.send().await.context("HTTP POST form failed")
    }

    /// POST request with JSON body
    pub async fn post_json(&self, path: &str, body: &serde_json::Value) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.post(&url).json(body);
        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }
        req.send().await.context("HTTP POST JSON failed")
    }
}
