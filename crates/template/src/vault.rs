use askama::Template;

#[derive(Template)]
#[template(path = "vault/vault_config.jinja", ext = "txt")]
pub struct VaultConfigTemplate {
    pub node_name: String,
}
