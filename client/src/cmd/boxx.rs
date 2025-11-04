use std::fs;
use std::path::Path;

use anyhow::Result;
use clap::Subcommand;

use data::{DeviceModels, Sherpa};

#[derive(Debug, Subcommand)]
pub enum BoxCommands {
    /// List all boxes
    List {
        /// Optional: List all boxes for a model
        #[arg(value_enum)]
        model: Option<DeviceModels>,
    },
}

/// Recursively list a directories contents.
fn list_directory_contents(path: &Path, indent: u8) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(path)?.filter_map(|e| e.ok()).collect();

    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if let Some(name) = path.file_name() {
            println!(
                "{:indent$}{}",
                "",
                name.to_string_lossy(),
                indent = indent as usize
            );
            if path.is_dir() {
                list_directory_contents(&path, indent + 2)?;
            }
        }
    }
    Ok(())
}

/// Parse the commands for Box
pub fn parse_box_commands(commands: &BoxCommands, config: &Sherpa) -> Result<()> {
    match commands {
        BoxCommands::List { model } => {
            if let Some(m) = model {
                let model_dir = format!("{}/{}", &config.boxes_dir, m);
                println!("{}", &model_dir);
                list_directory_contents(model_dir.as_ref(), 0)?;
            } else {
                println!("{}", &config.boxes_dir);
                list_directory_contents(config.boxes_dir.as_ref(), 0)?;
            }
        }
    }
    Ok(())
}
