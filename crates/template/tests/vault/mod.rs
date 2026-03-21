use askama::Template;

use template::VaultConfigTemplate;

// ============================================================================
// Expected configs
// ============================================================================

const EXPECTED_VAULT: &str = r#"storage "raft" {
  path       = "/vault/data"
  node_id = "vault01"
}

listener "tcp" {
  address         = "0.0.0.0:8200"
  tls_disable = true
}

disable_mlock  = false
api_addr       = "http://127.0.0.1:8200"
cluster_addr   = "https://127.0.0.1:8201"
ui             = true"#;

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_vault_config() {
    let t = VaultConfigTemplate {
        node_name: "vault01".to_string(),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_VAULT);
}
