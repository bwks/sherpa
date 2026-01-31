mod helper;
/// Integration tests for node_config CRUD operations
///
/// These tests require a running SurrealDB instance.
/// Run: surreal start --log trace --user sherpa --pass 'Everest1953!' memory
///
/// To run these tests:
/// - All node_config tests: cargo test --package db --test node_config
/// - Only CREATE tests: cargo test --package db create_tests
/// - Only READ tests: cargo test --package db read_tests
/// - Specific test: cargo test --package db test_get_node_config_by_id
mod node_config;

use helper::{create_test_config, setup_db};
