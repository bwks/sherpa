use std::fs;
use std::path::Path;

use anyhow::Result;
use clap::Subcommand;

use container::{docker_connection, list_images};
use shared::data::{NodeModel, Sherpa};
use shared::util::{
    copy_file, create_dir, create_symlink, file_exists, fix_permissions_recursive,
    term_msg_surround,
};

#[derive(Debug, Subcommand)]
pub enum ImageCommands {
    /// List all boxes
    List {
        /// Optional: List all boxes for a model
        #[arg(value_enum)]
        model: Option<NodeModel>,
        /// List container images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        containers: bool,
        /// List nanovm images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        nanovms: bool,
        /// List virtual machine images
        #[arg(long, action = clap::ArgAction::SetTrue)]
        virtual_machines: bool,
    },

    /// Import a disk image
    Import {
        /// Source path of the disk image
        #[arg(short, long)]
        src: String,
        /// Version of the device model
        #[arg(short, long)]
        version: String,
        /// Model of Device
        #[arg(short, long, value_enum)]
        model: NodeModel,
        /// Import the disk image as the latest version
        #[arg(long, action = clap::ArgAction::SetTrue)]
        latest: bool,
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

/// Parse the commands for Image
pub async fn parse_image_commands(commands: &ImageCommands, config: &Sherpa) -> Result<()> {
    match commands {
        ImageCommands::List {
            model,
            containers,
            nanovms,
            virtual_machines,
        } => {
            if let Some(m) = model {
                let model_dir = format!("{}/{}", &config.images_dir, m);
                println!("{}", &model_dir);
                list_directory_contents(model_dir.as_ref(), 0)?;
            } else if *containers {
                term_msg_surround("Container images");
                let docker_conn = docker_connection()?;
                list_images(&docker_conn).await?;
            } else if *nanovms || *virtual_machines {
                println!("NOT IMPLEMENTED")
            } else {
                println!("{}", &config.images_dir);
                list_directory_contents(config.images_dir.as_ref(), 0)?;
            }
        }
        ImageCommands::Import {
            src,
            version,
            model,
            latest,
        } => import(src, version, model, *latest, &config.images_dir)?,
    }
    Ok(())
}

fn import(
    src: &str,
    version: &str,
    model: &NodeModel,
    latest: bool,
    images_dir: &str,
) -> Result<()> {
    term_msg_surround("Importing disk image");

    if !file_exists(src) {
        anyhow::bail!("File does not exist: {}", src);
    }

    let dst_path = format!("{}/{}", images_dir, model);
    let dst_version_dir = format!("{dst_path}/{version}");
    let dst_latest_dir = format!("{dst_path}/latest");

    create_dir(&dst_version_dir)?;
    create_dir(&dst_latest_dir)?;

    let dst_version_disk = format!("{dst_version_dir}/virtioa.qcow2");

    if !file_exists(&dst_version_disk) {
        println!("Copying file from: {} to: {}", src, dst_version_disk);
        copy_file(src, &dst_version_disk)?;
        println!("Copied file from: {} to: {}", src, dst_version_disk);
    } else {
        println!("File already exists: {}", dst_version_disk);
    }

    if latest {
        let dst_latest_disk = format!("{dst_latest_dir}/virtioa.qcow2");
        println!("Symlinking file from: {} to: {}", src, dst_latest_disk);
        create_symlink(&dst_version_disk, &dst_latest_disk)?;
        println!("Symlinked file from: {} to: {}", src, dst_latest_disk);
    }

    println!("Setting base box files to read-only");
    fix_permissions_recursive(images_dir)?;

    Ok(())
}
