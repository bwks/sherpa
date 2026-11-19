//! Settings older releases wrote to `sherpa.toml` that sherpad no longer reads. The config
//! ignores fields it does not know, so a file that still has one loads; sherpad warns about it
//! at startup rather than editing the file.
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

/// The server-wide `[ztp_server]` table, which nothing read. A lab's own `[ztp_server]` in its
/// manifest is a different setting and is unaffected.
const ZTP_SERVER_TABLE: &str = "ztp_server";

/// Warn, once, when the server config at `path` still has the removed `[ztp_server]` table.
///
/// # Errors
///
/// Fails when the file cannot be read or is not TOML.
#[instrument(fields(path = %path.display()))]
pub fn warn_on_removed_settings(path: &Path) -> Result<()> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Unable to read server config: {}", path.display()))?;
    if has_ztp_server_table(&contents)? {
        tracing::warn!(
            table = ZTP_SERVER_TABLE,
            "The server config has a [{ZTP_SERVER_TABLE}] table, which sherpad no longer reads. \
             It is ignored and can be deleted from {}",
            path.display()
        );
    }
    Ok(())
}

/// Whether the server config text `contents` has a top-level `[ztp_server]` table.
fn has_ztp_server_table(contents: &str) -> Result<bool> {
    let table: toml::Table = toml::from_str(contents).context("Invalid server configuration")?;
    Ok(table.contains_key(ZTP_SERVER_TABLE))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::log_capture::capture_log;

    #[test]
    fn finds_the_ztp_server_table() {
        let contents = r#"server_ipv4 = "0.0.0.0"

[ztp_server]
enable = false
username = "sherpa"
password = "kept"
"#;
        assert!(has_ztp_server_table(contents).unwrap());
    }

    #[test]
    fn a_config_without_it_is_not_reported() {
        let contents = r#"server_ipv4 = "0.0.0.0"

[tls]
enabled = true
"#;
        assert!(!has_ztp_server_table(contents).unwrap());
    }

    #[test]
    fn a_key_of_that_name_inside_another_table_is_not_reported() {
        let contents = "[otel]\nztp_server = true\n";
        assert!(!has_ztp_server_table(contents).unwrap());
    }

    #[test]
    fn invalid_toml_is_an_error() {
        assert!(has_ztp_server_table("[ztp_server").is_err());
    }

    #[test]
    fn warns_without_error_for_a_file_with_the_table() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sherpa.toml");
        fs::write(&path, "[ztp_server]\nenable = false\n").unwrap();
        let (result, output) = capture_log(|| warn_on_removed_settings(&path));
        assert!(result.is_ok());
        assert!(output.contains("WARN"), "{output}");
        assert!(
            output.contains("[ztp_server] table, which sherpad no longer reads"),
            "{output}"
        );
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        assert!(warn_on_removed_settings(&directory.path().join("sherpa.toml")).is_err());
    }
}
