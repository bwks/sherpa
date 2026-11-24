use askama::Template;

#[derive(Template)]
#[template(path = "vault/vault_config.jinja", ext = "txt")]
pub struct VaultConfigTemplate {
    pub node_name: String,
    pub node_address: String,
    pub api_port: u16,
    pub cluster_port: u16,
    pub certs_dir: String,
}
