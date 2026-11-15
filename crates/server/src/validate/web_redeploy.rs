use anyhow::{Result, bail};

/// Refuse a web UI redeploy of `node`, a node of the lab's stored definition, when rebuilding it
/// needs what the server does not keep. The definition holds the manifest as the user wrote it,
/// without file contents or resolved values, and only the `sherpa` CLI, in the lab folder, can
/// read the files and variables these parts name: a `ztp.config`, `text_files`,
/// `binary_files`, `systemd_units`, startup and user scripts, `ssh.authorized_key_files`, and
/// `environment_variables` with a `$VAR`. Provisioners are refused too, as only the CLI runs
/// them. A node with none of these is rebuilt from its definition alone.
///
/// # Errors
///
/// Fails, naming the node and every missing part, when the node has any of them.
pub fn check_web_redeployable(node: &topology::Node) -> Result<()> {
    let listed = [
        (node.ztp.config.is_some(), "ztp.config"),
        (has_items(node.text_files.as_ref()), "text_files"),
        (has_items(node.binary_files.as_ref()), "binary_files"),
        (has_items(node.systemd_units.as_ref()), "systemd_units"),
        (has_items(node.startup_scripts.as_ref()), "startup_scripts"),
        (has_items(node.user_scripts.as_ref()), "user_scripts"),
        (
            has_items(node.ssh.authorized_key_files.as_ref()),
            "ssh.authorized_key_files",
        ),
        (
            node.environment_variables.iter().flatten().any(|entry| {
                entry
                    .split_once('=')
                    .is_some_and(|(_, value)| value.starts_with('$'))
            }),
            "environment_variables with a $VAR",
        ),
        (has_items(node.provisioners.as_ref()), "provisioners"),
    ];
    let missing: Vec<&str> = listed
        .into_iter()
        .filter_map(|(needed, part)| needed.then_some(part))
        .collect();
    if !missing.is_empty() {
        bail!(
            "Node '{name}' cannot be redeployed from the web UI: it needs {parts}, which the \
             server does not keep. Run `sherpa redeploy -n {name}` from the lab folder",
            name = node.name,
            parts = missing.join(", "),
        );
    }
    Ok(())
}

/// Whether the optional list has any items.
fn has_items<T>(list: Option<&Vec<T>>) -> bool {
    list.is_some_and(|list| !list.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node `dev01` of a manifest, from its TOML table without the `[[nodes]]` header.
    fn node(table: &str) -> topology::Node {
        let manifest: topology::Manifest = toml::from_str(&format!(
            "name = \"lab\"\n[[nodes]]\nname = \"dev01\"\nmodel = \"ubuntu_linux\"\n{table}"
        ))
        .unwrap();
        manifest.nodes[0].clone()
    }

    fn refusal(table: &str) -> String {
        check_web_redeployable(&node(table))
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn a_node_that_needs_no_contents_redeploys_from_the_web() {
        check_web_redeployable(&node("")).unwrap();
        check_web_redeployable(&node(
            "memory = 2048\nenvironment_variables = [\"MODE=lab\"]\nztp = { method = \"cdrom\" }\n\
             ssh = { authorized_keys = [\"ssh-ed25519 AAAA test\"] }\n",
        ))
        .unwrap();
    }

    #[test]
    fn each_kind_of_missing_content_is_named() {
        let text_file = "src = \"files/motd\"\ndst = \"/etc/motd\"\nuser = \"root\"\n\
                         group = \"root\"\npermissions = 644\n";
        let cases = [
            ("ztp = { config = \"configs/dev01.yaml\" }\n", "ztp.config"),
            (&format!("[[nodes.text_files]]\n{text_file}"), "text_files"),
            (
                "[[nodes.binary_files]]\nsource = \"files/blob.bin\"\n",
                "binary_files",
            ),
            (
                "[[nodes.systemd_units]]\nname = \"app.service\"\nsource = \"units/app.service\"\n\
                 enabled = true\n",
                "systemd_units",
            ),
            (
                "startup_scripts = [\"scripts/start.sh\"]\n",
                "startup_scripts",
            ),
            ("user_scripts = [\"scripts/user.sh\"]\n", "user_scripts"),
            (
                "ssh = { authorized_key_files = [{ source = \"keys/id.pub\" }] }\n",
                "ssh.authorized_key_files",
            ),
            (
                "environment_variables = [\"MODE=lab\", \"TOKEN=$LAB_TOKEN\"]\n",
                "environment_variables with a $VAR",
            ),
            (
                "[[nodes.provisioners]]\ntype = \"ssh\"\nscripts = [\"setup.sh\"]\n",
                "provisioners",
            ),
        ];
        for (table, part) in cases {
            assert_eq!(
                refusal(table),
                format!(
                    "Node 'dev01' cannot be redeployed from the web UI: it needs {part}, which \
                     the server does not keep. Run `sherpa redeploy -n dev01` from the lab folder"
                ),
                "{table}"
            );
        }
    }

    #[test]
    fn every_missing_part_is_named_at_once() {
        assert_eq!(
            refusal(
                "startup_scripts = [\"scripts/start.sh\"]\nztp = { config = \"configs/dev01.yaml\" }\n"
            ),
            "Node 'dev01' cannot be redeployed from the web UI: it needs ztp.config, \
             startup_scripts, which the server does not keep. Run `sherpa redeploy -n dev01` \
             from the lab folder"
        );
    }
}
