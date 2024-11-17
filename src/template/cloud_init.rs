use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use crate::core::konst::{
    SHERPA_CONFIG_DIR, SHERPA_PASSWORD, SHERPA_SSH_PUBLIC_KEY_FILE, SHERPA_USERNAME,
};
use crate::util::get_ssh_public_key;

#[derive(Serialize, Deserialize, Debug)]
pub struct CloudInitConfig {
    pub hostname: String,
    pub fqdn: String,
    pub ssh_pwauth: bool,
    pub users: Vec<CloudInitUser>,
}
impl CloudInitConfig {
    pub fn to_string(&self) -> Result<String> {
        // First serialize to regular YAML
        let yaml = serde_yaml::to_string(&self)?;

        // Prepend the #cloud-config comment
        let mut output = String::from("#cloud-config\n");
        output.push_str(&yaml);

        Ok(output)
    }
}
#[derive(Serialize, Deserialize, Debug)]
pub struct CloudInitUser {
    pub name: String,
    pub plain_text_passwd: String,
    pub lock_passwd: bool,
    pub ssh_authorized_keys: Vec<String>,
    pub sudo: String,
    pub groups: Vec<String>,
    pub shell: String,
}
impl CloudInitUser {
    pub fn default() -> Result<Self> {
        let ssh_key =
            get_ssh_public_key(&format!("{SHERPA_CONFIG_DIR}/{SHERPA_SSH_PUBLIC_KEY_FILE}"))?;
        Ok(Self {
            name: SHERPA_USERNAME.to_owned(),
            plain_text_passwd: SHERPA_PASSWORD.to_owned(),
            lock_passwd: false,
            ssh_authorized_keys: vec![format!("{} {}", ssh_key.algorithm, ssh_key.key.to_owned())],
            sudo: "ALL=(ALL) NOPASSWD:ALL".to_owned(),
            groups: vec!["sudo".to_owned()],
            shell: "/bin/bash".to_owned(),
        })
    }
}
