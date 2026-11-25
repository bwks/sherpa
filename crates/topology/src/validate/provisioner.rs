use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use tracing::instrument;

use shared::data::{ModelKind, NodeKind, OsVariant};
use shared::validators::{mode_from_octal_digits, validate_secret_file};

use crate::{ContainerProvisioner, Manifest, Provisioner, SshProvisioner, TopologyError};

/// Check node provisioners: ssh provisioners run only on Linux and BSD VMs and container
/// provisioners only on containers, and their paths, names and modes must be usable. At least
/// one node must be provisioned at a time.
///
/// # Errors
///
/// Fails on the first provisioner, path, name or mode that cannot be used.
#[instrument(skip(manifest), level = "debug")]
pub fn validate_provisioners(manifest: &Manifest) -> Result<()> {
    ensure!(
        manifest.provision_concurrency != Some(0),
        TopologyError::ProvisionConcurrencyZero
    );
    for node in &manifest.nodes {
        let provisioners = node.provisioners.as_deref().unwrap_or_default();
        if provisioners.is_empty() {
            continue;
        }
        let config = ModelKind::new(node.model, node.kind)?.node_config();
        // Env files and secret files from every provisioner on a node share one directory.
        let mut env_names = HashSet::new();
        let mut secret_names = HashSet::new();
        for provisioner in provisioners {
            match provisioner {
                Provisioner::Ssh(ssh) => {
                    ensure!(
                        config.vm().is_some_and(|vm| matches!(
                            vm.os_variant,
                            OsVariant::Linux | OsVariant::Bsd
                        )),
                        TopologyError::SshProvisionerNotSupported {
                            node: node.name.clone(),
                        }
                    );
                    validate_ssh(&node.name, ssh, &mut env_names)?;
                }
                Provisioner::Container(container) => {
                    ensure!(
                        config.kind() == NodeKind::Container,
                        TopologyError::ContainerProvisionerNotSupported {
                            node: node.name.clone(),
                        }
                    );
                    validate_container(&node.name, container, &mut secret_names)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_container(
    node: &str,
    container: &ContainerProvisioner,
    secret_names: &mut HashSet<String>,
) -> Result<()> {
    ensure!(
        !container.files.is_empty(),
        TopologyError::ContainerProvisionerEmpty {
            node: node.to_owned(),
        }
    );
    for file in &container.files {
        ensure!(
            !file.src.trim().is_empty(),
            TopologyError::SecretFileEmptySrc {
                node: node.to_owned(),
            }
        );
        validate_secret_file(&file.name, file.mode).with_context(|| format!("Node '{node}'"))?;
        ensure!(
            secret_names.insert(file.name.clone()),
            TopologyError::DuplicateSecretFile {
                node: node.to_owned(),
                name: file.name.clone(),
            }
        );
    }
    Ok(())
}

fn validate_ssh(node: &str, ssh: &SshProvisioner, env_names: &mut HashSet<String>) -> Result<()> {
    ensure!(
        !(ssh.files.is_empty() && ssh.env_files.is_empty() && ssh.scripts.is_empty()),
        TopologyError::SshProvisionerEmpty {
            node: node.to_owned(),
        }
    );
    for file in &ssh.files {
        ensure!(
            !file.src.trim().is_empty(),
            TopologyError::ProvisionerFileEmptySrc {
                node: node.to_owned(),
            }
        );
        ensure!(
            valid_destination(&file.dst),
            TopologyError::InvalidProvisionerDestination {
                node: node.to_owned(),
                dst: file.dst.clone(),
            }
        );
        mode_from_octal_digits(file.mode).with_context(|| TopologyError::ProvisionerFileMode {
            node: node.to_owned(),
            dst: file.dst.clone(),
        })?;
    }
    for env_file in &ssh.env_files {
        let name = env_file_name(env_file).ok_or_else(|| TopologyError::EnvFileNotAFile {
            node: node.to_owned(),
            env_file: env_file.clone(),
        })?;
        ensure!(
            env_names.insert(name.to_owned()),
            TopologyError::DuplicateEnvFile {
                node: node.to_owned(),
                name: name.to_owned(),
            }
        );
    }
    for script in &ssh.scripts {
        ensure!(
            !script.trim().is_empty(),
            TopologyError::EmptyScriptPath {
                node: node.to_owned(),
            }
        );
    }
    Ok(())
}

/// An absolute path, or one relative to the home directory with `~/`, naming a file.
fn valid_destination(dst: &str) -> bool {
    let ((Some(relative), _) | (None, Some(relative))) =
        (dst.strip_prefix("~/"), dst.strip_prefix('/'))
    else {
        return false;
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

/// A path part that needs no quoting in a remote shell command or an SFTP path.
fn safe_name(part: &str) -> bool {
    for byte in part.bytes() {
        if !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'@')) {
            return false;
        }
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
    use shared::SharedError;

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

    fn refusal(manifest: &Manifest) -> TopologyError {
        validate_provisioners(manifest)
            .unwrap_err()
            .downcast::<TopologyError>()
            .expect("a topology error")
    }

    fn rejection(model: &str, provisioner: &str) -> TopologyError {
        refusal(&manifest(model, provisioner))
    }

    fn container(model: &str, files: &str) -> Manifest {
        toml::from_str(&format!(
            r#"
            name = "lab"

            [[nodes]]
            name = "db01"
            model = "{model}"

            [[nodes.provisioners]]
            type = "container"
            {files}
            "#
        ))
        .unwrap()
    }

    fn secret(name: &str, mode: u32) -> String {
        format!(
            r#"
            [[nodes.provisioners.files]]
            src = "a.txt"
            name = "{name}"
            mode = {mode}
            "#
        )
    }

    #[test]
    fn manifest_provisioners_001_a_container_provisioner_on_a_virtual_machine_is_refused() {
        for model in ["ubuntu_linux", "free_bsd", "cisco_ios"] {
            assert_eq!(
                refusal(&container(model, &secret("token", 444))),
                TopologyError::ContainerProvisionerNotSupported {
                    node: "db01".to_owned(),
                },
                "{model}"
            );
        }
    }

    #[test]
    fn manifest_provisioners_003_two_secret_files_with_one_name_are_refused_naming_it() {
        let files = format!("{}{}", secret("token", 444), secret("token", 400));
        assert_eq!(
            refusal(&container("mongo_db", &files)),
            TopologyError::DuplicateSecretFile {
                node: "db01".to_owned(),
                name: "token".to_owned(),
            }
        );
    }

    fn file(dst: &str, mode: u32) -> String {
        format!(
            r#"
            [[nodes.provisioners.files]]
            src = "a.txt"
            dst = "{dst}"
            mode = {mode}
            "#
        )
    }

    #[test]
    fn manifest_provisioners_005_an_ssh_provisioner_is_taken_on_linux_and_bsd_virtual_machines() {
        let provisioner = format!(
            r#"
            env_files = ["~/a.env", "b.env"]
            scripts = ["./setup.sh"]
            {}{}
            "#,
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
    fn manifest_provisioners_006_an_ssh_provisioner_on_any_other_node_is_refused() {
        for model in [
            "cisco_ios",
            "cisco_iol",
            "windows_server",
            "nanos_unikernel",
        ] {
            assert_eq!(
                rejection(model, r#"scripts = ["./setup.sh"]"#),
                TopologyError::SshProvisionerNotSupported {
                    node: "dev01".to_owned(),
                },
                "{model}"
            );
        }
    }

    #[test]
    fn manifest_provisioners_002_a_provisioner_with_nothing_to_do_is_refused() {
        assert_eq!(
            rejection("ubuntu_linux", ""),
            TopologyError::SshProvisionerEmpty {
                node: "dev01".to_owned(),
            }
        );
        assert_eq!(
            refusal(&container("mongo_db", "")),
            TopologyError::ContainerProvisionerEmpty {
                node: "db01".to_owned(),
            }
        );
    }

    #[test]
    fn manifest_provisioners_008_a_dst_not_absolute_or_home_relative_of_plain_parts_is_refused() {
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
            assert_eq!(
                rejection("ubuntu_linux", &file(dst, 600)),
                TopologyError::InvalidProvisionerDestination {
                    node: "dev01".to_owned(),
                    dst: dst.to_owned(),
                }
            );
        }
    }

    #[test]
    fn manifest_provisioners_010_a_provisioner_file_mode_that_is_not_octal_digits_is_refused() {
        for mode in [800, 1777, 609] {
            let error = validate_provisioners(&manifest("ubuntu_linux", &file("~/token", mode)))
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<TopologyError>(),
                Some(&TopologyError::ProvisionerFileMode {
                    node: "dev01".to_owned(),
                    dst: "~/token".to_owned(),
                })
            );
            assert_eq!(
                error.root_cause().downcast_ref::<SharedError>(),
                Some(&SharedError::ModeNotOctal { digits: mode })
            );
        }
    }

    #[test]
    fn manifest_provisioners_004_an_empty_src_or_script_path_is_refused() {
        let error = rejection(
            "ubuntu_linux",
            r#"
            [[nodes.provisioners.files]]
            src = ""
            dst = "~/token"
            "#,
        );
        assert_eq!(
            error,
            TopologyError::ProvisionerFileEmptySrc {
                node: "dev01".to_owned(),
            }
        );
        let error = refusal(&container(
            "mongo_db",
            r#"
            [[nodes.provisioners.files]]
            src = " "
            name = "token"
            "#,
        ));
        assert_eq!(
            error,
            TopologyError::SecretFileEmptySrc {
                node: "db01".to_owned(),
            }
        );
        assert_eq!(
            rejection("ubuntu_linux", r#"scripts = [" "]"#),
            TopologyError::EmptyScriptPath {
                node: "dev01".to_owned(),
            }
        );
    }

    #[test]
    fn manifest_provisioners_011_an_env_file_needs_a_plain_name_of_its_own_on_the_node() {
        assert_eq!(
            rejection(
                "ubuntu_linux",
                r#"env_files = ["~/one/dev.env", "~/two/dev.env"]"#,
            ),
            TopologyError::DuplicateEnvFile {
                node: "dev01".to_owned(),
                name: "dev.env".to_owned(),
            }
        );
        for env_file in ["~/", "~/my dev.env", "~/dev$1.env"] {
            assert_eq!(
                rejection("ubuntu_linux", &format!(r#"env_files = ["{env_file}"]"#)),
                TopologyError::EnvFileNotAFile {
                    node: "dev01".to_owned(),
                    env_file: env_file.to_owned(),
                }
            );
        }
    }

    #[test]
    fn lab_settings_005_a_provision_concurrency_of_0_is_refused_and_1_is_taken() {
        let mut manifest = manifest("ubuntu_linux", r#"scripts = ["a.sh"]"#);
        manifest.provision_concurrency = Some(1);
        validate_provisioners(&manifest).unwrap();
        manifest.provision_concurrency = Some(0);
        assert_eq!(refusal(&manifest), TopologyError::ProvisionConcurrencyZero);
    }
}
