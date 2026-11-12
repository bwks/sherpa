//! The table of Sherpa's include lines in the user's SSH config.

use shared::util::{SshConfigInspectionEntry, TableLayout, render_table};

const SSH_CONFIG_HEADERS: [&str; 5] = ["Line", "Lab ID", "Status", "Path", "Reason"];

/// Renders Sherpa SSH config inspection results.
#[must_use]
pub fn render_ssh_config_inspection_table(entries: &[SshConfigInspectionEntry]) -> String {
    let rows: Vec<[String; 5]> = entries.iter().map(ssh_config_cells).collect();
    render_table(
        SSH_CONFIG_HEADERS,
        &rows,
        &TableLayout {
            title: Some("Sherpa SSH Config Includes"),
            ..TableLayout::default()
        },
    )
}

fn ssh_config_cells(entry: &SshConfigInspectionEntry) -> [String; 5] {
    [
        entry.line_number.to_string(),
        entry.lab_id.as_deref().unwrap_or("-").to_string(),
        entry.status.to_string(),
        entry.include_path.clone(),
        entry.reason.clone(),
    ]
}
