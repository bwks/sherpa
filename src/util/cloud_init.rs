use std::fs::{self, File};
use std::io::Write;
use std::process::Command;

use anyhow::Result;
use askama::Template;

use crate::core::konst::{
    CLOUD_INIT_DIR, CLOUD_INIT_ISO, CLOUD_INIT_META_DATA, CLOUD_INIT_USER_DATA, CONFIG_DIR,
    TEMP_DIR,
};
use crate::libvirt::CloudInitTemplate;
use crate::model::User;
use crate::util::expand_path;

/// Create a cloud-init ISO file.
/// This wraps the `genisoimage` command to create a cloud-init ISO file.
/// `genisoimage` must be installed on the system.
///
/// TODO:
/// This function creates temp directory and files in the /tmp
/// directory. This is dogwater, and in the future, the plan is to
/// implement this functionality in pure Rust.
pub fn create_cloud_init_iso(users: Vec<User>) -> Result<()> {
    // Create a temporary directory
    fs::create_dir_all(&format!("{TEMP_DIR}/{CLOUD_INIT_DIR}"))?;

    let cloud_init_template = CloudInitTemplate { users };
    let cloud_init_rendered = cloud_init_template.render()?;

    let mut file = File::create(format!(
        "{TEMP_DIR}/{CLOUD_INIT_DIR}/{CLOUD_INIT_USER_DATA}"
    ))?;
    file.write_all(cloud_init_rendered.as_bytes())?;

    // Metadata file must exists, but it can be empty.
    // Create meta-data file (empty in this case)
    File::create(format!(
        "{TEMP_DIR}/{CLOUD_INIT_DIR}/{CLOUD_INIT_META_DATA}"
    ))?;

    let iso_path = expand_path(&format!("{CONFIG_DIR}/{CLOUD_INIT_ISO}"));

    // Create ISO using genisoimage
    Command::new("genisoimage")
        .args(&[
            "-output",
            &iso_path,
            "-volid",
            "cidata",
            "-joliet",
            "-rock",
            "--input-charset",
            "utf-8",
            &format!("{TEMP_DIR}/{CLOUD_INIT_DIR}"),
        ])
        .status()?;
    println!("cloud-init ISO created successfully: {iso_path}");

    // Clean up temp files
    fs::remove_dir_all(&format!("{TEMP_DIR}/{CLOUD_INIT_DIR}"))?;

    Ok(())
}
