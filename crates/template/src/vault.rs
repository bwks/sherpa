use askama::Template;

#[derive(Template)]
#[template(path = "vault/vault_config.jinja", ext = "txt")]
pub struct VaultConfigTemplate {
    pub node_name: String,
    /// Management address the API and cluster listeners advertise.
    pub node_address: String,
    pub api_port: u16,
    pub cluster_port: u16,
    /// Directory holding the node's `node.crt` and `node.key` from the lab CA.
    pub certs_dir: String,
}
