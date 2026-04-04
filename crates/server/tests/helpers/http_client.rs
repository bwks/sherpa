use anyhow::{Context, Result};
use reqwest::{Client, Response, StatusCode};
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
        let jar = Arc::new(reqwest::cookie::Jar::default());
        let client = Client::builder()
            .cookie_provider(jar)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();

        Self {
            client,
            base_url: format!("http://{}", addr),
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

    /// DELETE request
    pub async fn delete(&self, path: &str) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.delete(&url);
        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }
        req.send().await.context("HTTP DELETE failed")
    }

    /// Login via the HTML form endpoint and store session cookie
    pub async fn login_form(&self, username: &str, password: &str) -> Result<StatusCode> {
        let resp = self
            .post_form("/login", &[("username", username), ("password", password)])
            .await?;
        Ok(resp.status())
    }
}
