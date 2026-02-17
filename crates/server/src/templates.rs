use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use shared::data::LabSummary;

/// Main dashboard page template
#[derive(Template)]
#[template(path = "dashboard.html.jinja")]
pub struct DashboardTemplate {
    pub username: String,
}

impl IntoResponse for DashboardTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Labs grid partial template - displays all labs
#[derive(Template)]
#[template(path = "partials/labs-grid.html.jinja")]
pub struct LabsGridTemplate {
    pub labs: Vec<LabSummary>,
}

impl IntoResponse for LabsGridTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Empty state partial template - shown when no labs exist
#[derive(Template)]
#[template(path = "partials/empty-state.html.jinja")]
pub struct EmptyStateTemplate {
    pub username: String,
}

impl IntoResponse for EmptyStateTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

/// Error partial template - displays error messages
#[derive(Template)]
#[template(path = "partials/error.html.jinja")]
pub struct ErrorTemplate {
    pub message: String,
}

impl IntoResponse for ErrorTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}
