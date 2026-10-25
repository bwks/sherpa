//! Client-side provisioners, run by the `sherpa` CLI on a node after it is ready.
//!
//! Provisioners carry data that must not reach the Sherpa server, so `Node::provisioners`
//! is never serialized: the client does not send it and the server never stores it.
use serde_derive::{Deserialize, Serialize};

use shared::konst::PROVISIONER_FILE_DEFAULT_MODE;

/// One provisioner on a node, selected by its `type`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Provisioner {
    /// Copy files and run scripts over SSH, as the node's login user.
    Ssh(SshProvisioner),
}

/// Copies files with `scp` and runs scripts over SSH, in this order: `files`, `env_files`,
/// `scripts`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SshProvisioner {
    /// Client files copied to the node.
    #[serde(default)]
    pub files: Vec<ProvisionerFile>,
    /// Client env files (`KEY=value` lines) copied to `~/.env.d/`, which login shells load.
    #[serde(default)]
    pub env_files: Vec<String>,
    /// Client scripts run on the node in a login shell, through their shebang.
    #[serde(default)]
    pub scripts: Vec<String>,
}

/// A client file copied to a node.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionerFile {
    /// Path on the client: `~` is expanded and relative paths start at the manifest.
    pub src: String,
    /// Path on the node: absolute, or `~/` relative to the login user's home.
    pub dst: String,
    /// File mode as octal digits, for example `600`.
    #[serde(default = "default_file_mode")]
    pub mode: u32,
}

fn default_file_mode() -> u32 {
    PROVISIONER_FILE_DEFAULT_MODE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Manifest;

    const MANIFEST: &str = r#"
name = "lab"

[[nodes]]
name = "dev01"
model = "ubuntu_linux"

[[nodes.provisioners]]
type = "ssh"
env_files = ["~/.secrets/dev.env"]
scripts = ["./setup.sh"]

[[nodes.provisioners.files]]
src = "~/hosts.yml"
dst = "~/.config/gh/hosts.yml"
"#;

    fn parse_error(manifest: &str) -> String {
        toml::from_str::<Manifest>(manifest)
            .unwrap_err()
            .to_string()
    }

    fn ssh(manifest: &Manifest) -> &SshProvisioner {
        let provisioners = manifest.nodes[0].provisioners.as_ref().unwrap();
        match &provisioners[0] {
            Provisioner::Ssh(ssh) => ssh,
        }
    }

    #[test]
    fn ssh_provisioner_parses_from_a_node() {
        let manifest: Manifest = toml::from_str(MANIFEST).unwrap();
        let ssh = ssh(&manifest);
        assert_eq!(ssh.files[0].src, "~/hosts.yml");
        assert_eq!(ssh.files[0].dst, "~/.config/gh/hosts.yml");
        assert_eq!(ssh.files[0].mode, 600);
        assert_eq!(ssh.env_files, vec!["~/.secrets/dev.env"]);
        assert_eq!(ssh.scripts, vec!["./setup.sh"]);
    }

    #[test]
    fn provisioners_are_never_serialized() {
        let manifest: Manifest = toml::from_str(MANIFEST).unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        let saved = toml::to_string_pretty(&manifest).unwrap();
        for output in [json, saved] {
            assert!(!output.contains("provisioners"), "{output}");
            assert!(!output.contains("secrets/dev.env"), "{output}");
        }
    }

    #[test]
    fn unknown_provisioner_type_is_rejected() {
        let error = parse_error(&MANIFEST.replace(r#"type = "ssh""#, r#"type = "telnet""#));
        assert!(error.contains("telnet"), "{error}");
    }

    #[test]
    fn unknown_ssh_provisioner_field_is_rejected() {
        let error = parse_error(&MANIFEST.replace("scripts =", "commands ="));
        assert!(error.contains("commands"), "{error}");
    }

    #[test]
    fn unknown_file_field_is_rejected() {
        let error = parse_error(&MANIFEST.replace(
            r#"src = "~/hosts.yml""#,
            "owner = \"root\"\nsrc = \"~/hosts.yml\"",
        ));
        assert!(error.contains("owner"), "{error}");
    }
}
