use anyhow::Result;

use crate::data::DeviceModels;
use crate::util::{
    copy_file, create_dir, create_symlink, file_exists, fix_permissions_recursive,
    term_msg_surround,
};

pub fn import(
    src: &str,
    version: &str,
    model: &DeviceModels,
    latest: bool,
    boxes_dir: &str,
) -> Result<()> {
    term_msg_surround("Importing disk image");

    if !file_exists(src) {
        anyhow::bail!("File does not exist: {}", src);
    }

    let dst_path = format!("{}/{}", boxes_dir, model);
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
    fix_permissions_recursive(boxes_dir)?;

    Ok(())
}
