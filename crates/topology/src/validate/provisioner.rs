use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use tracing::instrument;

use shared::data::{NodeConfig, NodeKind, OsVariant};
use shared::konst::NODE_USER_ENV_DIR;

use crate::{Manifest, Provisioner, SshProvisioner};

/// Check node provisioners: they run only on Linux and BSD VMs, and their paths and modes must
/// be usable on the node.
#[instrument(skip(manifest), level = "debug")]
pub fn validate_provisioners(manifest: &Manifest) -> Result<()> {
    for node in &manifest.nodes {
        let provisioners = node.provisioners.as_deref().unwrap_or_default();
        if provisioners.is_empty() {
            continue;
        }
        let config = NodeConfig::get_model(node.model);
        ensure!(
            config.kind == NodeKind::VirtualMachine
                && matches!(config.os_variant, OsVariant::Linux | OsVariant::Bsd),
            "Node '{}' has provisioners, which run only on Linux and BSD VMs",
            node.name
        );
        // Env files from every provisioner on a node share the node's env directory.
        let mut env_names = HashSet::new();
        for provisioner in provisioners {
            match provisioner {
                Provisioner::Ssh(ssh) => validate_ssh(&node.name, ssh, &mut env_names)?,
            }
        }
    }
    Ok(())
}

fn validate_ssh(node: &str, ssh: &SshProvisioner, env_names: &mut HashSet<String>) -> Result<()> {
    ensure!(
        !(ssh.files.is_empty() && ssh.env_files.is_empty() && ssh.scripts.is_empty()),
        "Node '{node}': an ssh provisioner has nothing to do; add files, env_files or scripts"
    );
    for file in &ssh.files {
        ensure!(
            !file.src.trim().is_empty(),
            "Node '{node}': a provisioner file has an empty src"
        );
        ensure!(
            valid_destination(&file.dst),
            "Node '{node}': provisioner file dst '{}' must be an absolute path or start with ~/, \
             name a file, use only letters, digits and . _ - + @, and not contain empty, . or .. \
             parts",
            file.dst
        );
        ensure!(
            valid_mode(file.mode),
            "Node '{node}': provisioner file mode {} for '{}' must be octal digits up to 777",
            file.mode,
            file.dst
        );
    }
    for env_file in &ssh.env_files {
        let name = env_file_name(env_file).with_context(|| {
            format!("Node '{node}': env file '{env_file}' does not name a file")
        })?;
        ensure!(
            env_names.insert(name.to_owned()),
            "Node '{node}': two env files are named '{name}', and both would be copied to \
             ~/{NODE_USER_ENV_DIR}/{name}"
        );
    }
    for script in &ssh.scripts {
        ensure!(
            !script.trim().is_empty(),
            "Node '{node}': a provisioner script path is empty"
        );
    }
    Ok(())
}

/// An absolute path, or one relative to the home directory with `~/`, naming a file.
fn valid_destination(dst: &str) -> bool {
    let relative = match (dst.strip_prefix("~/"), dst.strip_prefix('/')) {
        (Some(relative), _) | (None, Some(relative)) => relative,
        (None, None) => return false,
    };
    if relative.is_empty() {
        return false;
    }
    for part in relative.split('/') {
        if part.is_empty() || part == "." || part == ".." || !safe_name(part) {
            return false;
        }
    }
    true
}

/// A path part that needs no quoting in a remote shell or `scp` path.
fn safe_name(part: &str) -> bool {
    for byte in part.bytes() {
        if !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'@')) {
            return false;
        }
    }
    true
}

/// A mode written as octal digits, such as `600`.
fn valid_mode(mode: u32) -> bool {
    if mode > 777 {
        return false;
    }
    let mut rest = mode;
    while rest > 0 {
        if rest % 10 > 7 {
            return false;
        }
        rest /= 10;
    }
    true
}

/// The name an env file gets in the node's env directory: its file name on the client.
fn env_file_name(env_file: &str) -> Option<&str> {
    if env_file.ends_with('/') || env_file.ends_with('\\') {
        return None;
    }
    let name = Path::new(env_file).file_name()?.to_str()?;
    (name != "~" && safe_name(name)).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(model: &str, provisioner: &str) -> Manifest {
        toml::from_str(&format!(
            r#"
name = "lab"

[[nodes]]
name = "dev01"
model = "{model}"

[[nodes.provisioners]]
type = "ssh"
{provisioner}
"#
        ))
        .unwrap()
    }

    fn rejection(model: &str, provisioner: &str) -> String {
        format!(
            "{:#}",
            validate_provisioners(&manifest(model, provisioner)).unwrap_err()
        )
    }

    fn file(dst: &str, mode: u32) -> String {
        format!("[[nodes.provisioners.files]]\nsrc = \"a.txt\"\ndst = \"{dst}\"\nmode = {mode}\n")
    }

    #[test]
    fn linux_and_bsd_vms_accept_valid_provisioners() {
        let provisioner = format!(
            "env_files = [\"~/a.env\", \"b.env\"]\nscripts = [\"./setup.sh\"]\n{}{}",
            file("~/.config/app/token", 600),
            file("/tmp/token", 640)
        );
        for model in [
            "ubuntu_linux",
            "alpine_linux",
            "flatcar_linux",
            "free_bsd",
            "open_bsd",
        ] {
            validate_provisioners(&manifest(model, &provisioner)).unwrap();
        }
    }

    #[test]
    fn nodes_without_provisioners_pass() {
        let manifest: Manifest =
            toml::from_str("name = \"lab\"\n[[nodes]]\nname = \"r1\"\nmodel = \"cisco_iosv\"\n")
                .unwrap();
        validate_provisioners(&manifest).unwrap();
    }

    #[test]
    fn other_nodes_are_rejected() {
        for model in [
            "cisco_iosv",
            "arista_ceos",
            "windows_server",
            "nanos_unikernel",
        ] {
            let error = rejection(model, "scripts = [\"./setup.sh\"]");
            assert!(error.contains("dev01"), "{error}");
            assert!(error.contains("Linux and BSD VMs"), "{error}");
        }
    }

    #[test]
    fn a_provisioner_must_do_something() {
        let error = rejection("ubuntu_linux", "");
        assert!(error.contains("nothing to do"), "{error}");
    }

    #[test]
    fn file_destinations_must_be_absolute_or_home_relative() {
        for dst in [
            "token",
            "~",
            "~/",
            "~alice/token",
            "",
            "/opt/../etc/token",
            "~/a//b",
            "/tmp/dir/",
            "~/my token",
            "~/$(id)",
            "/tmp/a;b",
            "~/it's",
        ] {
            let error = rejection("ubuntu_linux", &file(dst, 600));
            assert!(error.contains("dst"), "{dst}: {error}");
        }
    }

    #[test]
    fn file_modes_must_be_octal_digits() {
        for mode in [800, 1777, 609] {
            let error = rejection("ubuntu_linux", &file("~/token", mode));
            assert!(error.contains("mode"), "{mode}: {error}");
        }
    }

    #[test]
    fn empty_sources_and_scripts_are_rejected() {
        let error = rejection(
            "ubuntu_linux",
            "[[nodes.provisioners.files]]\nsrc = \"\"\ndst = \"~/token\"\n",
        );
        assert!(error.contains("src"), "{error}");
        let error = rejection("ubuntu_linux", "scripts = [\" \"]");
        assert!(error.contains("script"), "{error}");
    }

    #[test]
    fn env_file_names_must_be_unique_on_a_node() {
        let error = rejection(
            "ubuntu_linux",
            "env_files = [\"~/one/dev.env\", \"~/two/dev.env\"]",
        );
        assert!(error.contains("dev.env"), "{error}");
        for env_file in ["~/", "~/my dev.env", "~/dev$1.env"] {
            let error = rejection("ubuntu_linux", &format!("env_files = [\"{env_file}\"]"));
            assert!(error.contains("env file"), "{env_file}: {error}");
        }
    }
}
