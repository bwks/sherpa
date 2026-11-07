//! The server's integration tests, in one test binary so the helpers are shared without dead
//! code. Run one area by its module path, e.g. `cargo test -p sherpad --test integration
//! lab_lifecycle::`. Tests that need Docker, libvirt or host network rights are `#[ignore]`;
//! see `tests/README.md`.

mod auth;
mod helpers;
mod image_management;
mod lab_authorization;
mod lab_lifecycle;
mod security;
mod user_management;
mod websocket;
