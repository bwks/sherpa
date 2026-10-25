//! Remote shell commands the SSH provisioner runs on a node, as the node's login user.
//!
//! Each command is a POSIX `sh` string. Secret data never appears in one: file contents go
//! through `scp`, and script contents through the session's stdin.
use shared::konst::{NODE_USER_ENV_DIR, NODE_USER_ENV_FILE};

/// Quote a value as one POSIX shell word.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// A destination as a shell word on the node: `~/` paths are relative to `$HOME`.
pub fn remote_path(dst: &str) -> String {
    match dst.strip_prefix("~/") {
        Some(relative) => format!(r#""$HOME"/{}"#, shell_quote(relative)),
        None => shell_quote(dst),
    }
}

/// A destination as an `scp` path: `~/` paths become relative, which `scp` resolves against the
/// login user's home.
pub fn scp_path(dst: &str) -> String {
    dst.strip_prefix("~/").unwrap_or(dst).to_string()
}

/// Create the destination's missing parents and an empty destination file, private to the
/// login user, so the copied contents are never readable by others. An existing file keeps its
/// contents and is made private.
pub fn prepare_file_command(dst: &str) -> String {
    format!(
        r#"umask 077 && f={} && mkdir -p "$(dirname "$f")" && : >> "$f" && chmod 600 "$f""#,
        remote_path(dst)
    )
}

/// Give a copied file its final mode, written as octal digits.
pub fn finish_file_command(dst: &str, mode: u32) -> String {
    format!("chmod {mode:03} {}", remote_path(dst))
}

/// The destination of an env file on the node.
pub fn env_file_destination(name: &str) -> String {
    format!("~/{NODE_USER_ENV_DIR}/{name}")
}

/// Shell that exports the variables in each readable file, in the order given.
fn source_env_files(files: &str) -> String {
    format!(
        r#"for sherpa_env_file in {files}; do [ -r "$sherpa_env_file" ] && {{ set -a; . "$sherpa_env_file"; set +a; }}; done; unset sherpa_env_file"#
    )
}

/// Make login shells load every env file in the env directory, exporting its variables. The
/// line goes in the login file bash reads, `~/.bash_profile`, otherwise `~/.profile`, once.
pub fn env_loader_command() -> String {
    let line = shell_quote(&source_env_files(&format!(
        r#""$HOME"/{NODE_USER_ENV_DIR}/*"#
    )));
    format!(
        r#"f="$HOME/.bash_profile"; [ -f "$f" ] || f="$HOME/.profile"; grep -qxF {line} "$f" 2>/dev/null || printf '%s\n' {line} >> "$f""#
    )
}

/// Run a script read from stdin, through its shebang, with the variables from the manifest's
/// `environment_variables` and the env directory exported. The script is written to a private
/// temporary file, which is removed afterwards; the command exits with the script's status.
pub fn script_wrapper_command() -> String {
    let env = source_env_files(&format!(
        r#""$HOME"/{NODE_USER_ENV_FILE} "$HOME"/{NODE_USER_ENV_DIR}/*"#
    ));
    format!(
        r#"t=$(mktemp "${{TMPDIR:-/tmp}}/sherpa-script.XXXXXXXX") || exit 1; trap 'rm -f "$t"' EXIT; cat > "$t" && chmod 700 "$t" || exit 1; {env}; "$t" < /dev/null"#
    )
}

/// Wait for cloud-init to finish first-boot work on nodes that have it. Its exit status is
/// ignored, because a degraded boot still finishes.
pub fn cloud_init_wait_command() -> String {
    "command -v cloud-init >/dev/null 2>&1 && cloud-init status --wait >/dev/null 2>&1; exit 0"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{Command, Output, Stdio};

    use tempfile::tempdir;

    /// Run `command` with `sh -c` as a remote login shell would, with `home` as `$HOME`.
    fn run(home: &Path, command: &str, stdin: &str) -> Output {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .env("HOME", home)
            .env("TMPDIR", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn paths_resolve_home_relative_destinations() {
        assert_eq!(remote_path("~/.config/a"), r#""$HOME"/'.config/a'"#);
        assert_eq!(remote_path("/etc/a"), "'/etc/a'");
        assert_eq!(scp_path("~/.config/a"), ".config/a");
        assert_eq!(scp_path("/etc/a"), "/etc/a");
    }

    #[test]
    fn a_file_is_private_before_its_contents_arrive() {
        let home = tempdir().unwrap();
        let output = run(
            home.path(),
            &prepare_file_command("~/.config/app/token"),
            "",
        );
        assert!(output.status.success(), "{output:?}");
        let file = home.path().join(".config/app/token");
        assert_eq!(fs::read_to_string(&file).unwrap(), "");
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(&home.path().join(".config")), 0o700);
    }

    #[test]
    fn an_existing_file_keeps_its_contents_and_is_made_private() {
        let home = tempdir().unwrap();
        let file = home.path().join("token");
        fs::write(&file, "old").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let output = run(home.path(), &prepare_file_command("~/token"), "");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "old");
        assert_eq!(mode(&file), 0o600);
    }

    #[test]
    fn a_copied_file_gets_its_final_mode() {
        let home = tempdir().unwrap();
        let dst = format!("{}/srv/token", home.path().display());
        run(home.path(), &prepare_file_command(&dst), "");
        let output = run(home.path(), &finish_file_command(&dst, 640), "");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(mode(Path::new(&dst)), 0o640);
        run(home.path(), &finish_file_command(&dst, 44), "");
        assert_eq!(mode(Path::new(&dst)), 0o044);
    }

    #[test]
    fn the_env_loader_is_added_once_to_the_login_file_bash_reads() {
        let home = tempdir().unwrap();
        fs::write(home.path().join(".bash_profile"), "# mine\n").unwrap();
        for _ in 0..2 {
            let output = run(home.path(), &env_loader_command(), "");
            assert!(output.status.success(), "{output:?}");
        }
        let profile = fs::read_to_string(home.path().join(".bash_profile")).unwrap();
        assert_eq!(profile.matches(NODE_USER_ENV_DIR).count(), 1, "{profile}");
        assert!(!home.path().join(".profile").exists());
    }

    #[test]
    fn login_shells_export_env_files() {
        let home = tempdir().unwrap();
        run(home.path(), &env_loader_command(), "");
        let env_dir = home.path().join(NODE_USER_ENV_DIR);
        fs::create_dir(&env_dir).unwrap();
        fs::write(env_dir.join("dev.env"), "TOKEN='it''s secret'\n").unwrap();
        let output = run(
            home.path(),
            r#". "$HOME/.profile"; sh -c 'printf %s "$TOKEN"'"#,
            "",
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "its secret");
    }

    #[test]
    fn a_script_from_stdin_runs_with_the_node_environment_and_its_status() {
        let home = tempdir().unwrap();
        fs::write(
            home.path().join(NODE_USER_ENV_FILE),
            "export FROM_MANIFEST=a\n",
        )
        .unwrap();
        let env_dir = home.path().join(NODE_USER_ENV_DIR);
        fs::create_dir(&env_dir).unwrap();
        fs::write(env_dir.join("dev.env"), "FROM_ENV_FILE=b\n").unwrap();
        let script = "#!/bin/sh\nprintf '%s-%s' \"$FROM_MANIFEST\" \"$FROM_ENV_FILE\"\nexit 3\n";
        let output = run(home.path(), &script_wrapper_command(), script);
        assert_eq!(String::from_utf8_lossy(&output.stdout), "a-b");
        assert_eq!(output.status.code(), Some(3));
        for entry in fs::read_dir(home.path()).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(
                !name.to_string_lossy().starts_with("sherpa-script"),
                "temporary script left behind: {name:?}"
            );
        }
    }

    #[test]
    fn a_script_without_a_shebang_runs_with_sh() {
        let home = tempdir().unwrap();
        let output = run(home.path(), &script_wrapper_command(), "printf ok\n");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "ok");
        assert!(output.status.success());
    }
}
