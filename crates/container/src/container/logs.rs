use anyhow::{Context, Result};
use bollard::Docker;
use bollard::query_parameters::LogsOptions;
use futures_util::TryStreamExt;
use tracing::instrument;

/// The last `lines` lines a container wrote to stdout and stderr, as text with control
/// characters other than newlines and tabs removed, so it can be shown to a user.
///
/// # Errors
///
/// Fails when Docker cannot read the container's logs.
#[instrument(skip(docker), fields(%container_name), level = "debug")]
pub async fn container_log_tail(
    docker: &Docker,
    container_name: &str,
    lines: u32,
) -> Result<String> {
    let mut stream = docker.logs(
        container_name,
        Some(LogsOptions {
            stdout: true,
            stderr: true,
            tail: lines.to_string(),
            ..Default::default()
        }),
    );
    let mut raw = Vec::new();
    while let Some(output) = stream
        .try_next()
        .await
        .with_context(|| format!("failed to read the logs of container {container_name}"))?
    {
        raw.extend_from_slice(&output.into_bytes());
    }
    Ok(printable_log(&String::from_utf8_lossy(&raw)))
}

/// `log` without carriage returns and other control characters, which a terminal would act on,
/// and without trailing blank lines.
fn printable_log(log: &str) -> String {
    let text: String = log
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect();
    text.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_keeps_its_lines_without_control_characters() {
        let log = "first line\r\n\u{1b}[1msecond\u{1b}[0m\tline\r\n\r\n";
        assert_eq!(printable_log(log), "first line\n[1msecond[0m\tline");
    }

    #[test]
    fn an_empty_log_is_empty() {
        assert_eq!(printable_log("\r\n"), "");
    }
}
