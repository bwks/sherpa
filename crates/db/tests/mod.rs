mod helper;
/// Integration tests for node_config CRUD operations
///
/// These tests require a running SurrealDB instance.
/// Run: surreal start --log trace --user sherpa --pass 'Everest1953!' memory
///
/// Each test uses a unique namespace for isolation, preventing test data pollution.
///
/// To run these tests:
/// - All node_config tests: cargo test --package db --test node_config
/// - Only CREATE tests: cargo test --package db create_tests
/// - Only READ tests: cargo test --package db read_tests
/// - Specific test: cargo test --package db test_get_node_config_by_id
mod node_config;

/// Integration tests for lab CRUD operations
///
/// These tests require a running SurrealDB instance.
/// Run: surreal start --log trace --user sherpa --pass 'Everest1953!' memory
///
/// To run these tests:
/// - All lab tests: cargo test --package db lab -- --ignored
/// - Only CREATE tests: cargo test --package db lab::create_tests -- --ignored
/// - Only READ tests: cargo test --package db lab::read_tests -- --ignored
/// - Only UPDATE tests: cargo test --package db lab::update_tests -- --ignored
/// - Only DELETE tests: cargo test --package db lab::delete_tests -- --ignored
mod lab;

/// Integration tests for user CRUD operations
///
/// These tests require a running SurrealDB instance.
/// Run: surreal start --log trace --user sherpa --pass 'Everest1953!' memory
///
/// To run these tests:
/// - All user tests: cargo test --package db --test user
/// - Only CREATE tests: cargo test --package db user::create_tests
/// - Only READ tests: cargo test --package db user::read_tests
/// - Only UPDATE tests: cargo test --package db user::update_tests
/// - Only DELETE tests: cargo test --package db user::delete_tests
mod user;

use helper::{create_test_config, setup_db, teardown_db};
