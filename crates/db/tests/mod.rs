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

/// Integration tests for node CRUD operations
///
/// These tests require a running SurrealDB instance.
/// Run: surreal start --log trace --user sherpa --pass 'Everest1953!' memory
///
/// To run these tests:
/// - All node tests: cargo test --package db node -- --ignored --test-threads=1
/// - Only CREATE tests: cargo test --package db node::create_tests -- --ignored --test-threads=1
/// - Only READ tests: cargo test --package db node::read_tests -- --ignored --test-threads=1
/// - Only UPDATE tests: cargo test --package db node::update_tests -- --ignored --test-threads=1
/// - Only DELETE tests: cargo test --package db node::delete_tests -- --ignored --test-threads=1
mod node;

/// Integration tests for link CRUD operations
///
/// These tests require a running SurrealDB instance.
/// Run: surreal start --log trace --user sherpa --pass 'Everest1953!' memory
///
/// To run these tests:
/// - All link tests: cargo test --package db link -- --ignored --test-threads=1
/// - Only CREATE tests: cargo test --package db link::create_tests -- --ignored --test-threads=1
/// - Only READ tests: cargo test --package db link::read_tests -- --ignored --test-threads=1
/// - Only UPDATE tests: cargo test --package db link::update_tests -- --ignored --test-threads=1
/// - Only DELETE tests: cargo test --package db link::delete_tests -- --ignored --test-threads=1
mod link;

use helper::{create_test_config, create_test_node_with_model, setup_db, teardown_db};
