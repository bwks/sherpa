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
  address       = "0.0.0.0:8200"
  tls_cert_file = "/etc/sherpa/certs/node.crt"
  tls_key_file  = "/etc/sherpa/certs/node.key"
}

disable_mlock  = false
api_addr       = "https://172.31.1.21:8200"
cluster_addr   = "https://172.31.1.21:8201"
ui             = true"#;

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_vault_config() {
    let t = VaultConfigTemplate {
        node_name: "vault01".to_string(),
        node_address: "172.31.1.21".to_string(),
        api_port: 8200,
        cluster_port: 8201,
        certs_dir: "/etc/sherpa/certs".to_string(),
    };
    let output = t.render().expect("template renders");
    assert_eq!(output, EXPECTED_VAULT);
}
