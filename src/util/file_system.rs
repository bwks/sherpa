use std::fs;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::Result;

/// Create a directory, expanding ~ if it's passed
pub fn create_dir(dir_path: &str) -> Result<()> {
    fs::create_dir_all(dir_path)?;
    println!("Directory path created successfully: {dir_path}");

    Ok(())
}

/// Create a file, expanding ~ if it's passed
pub fn create_file(file_path: &str, contents: String) -> Result<()> {
    fs::write(file_path, contents)?;
    println!("File created successfully: {file_path}");

    Ok(())
}

/// Check for file existence, expanding ~ if it's passed
pub fn file_exists(file_path: &str) -> bool {
    let path = Path::new(file_path);
    path.exists() && path.is_file()
}

/// Check for directory existence, expanding ~ if it's passed
pub fn dir_exists(dir_path: &str) -> bool {
    let path = Path::new(dir_path);
    path.exists() && path.is_dir()
}

/// Expand a path if it's passed with ~
pub fn expand_path(path: &str) -> String {
    let expanded_path = shellexpand::tilde(path);
    let full_path = Path::new(expanded_path.as_ref());
    full_path.display().to_string()
}

/// Copy a file from a source to a destination.
/// This will overwrite the destination file if it exists.
pub fn copy_file(src: &str, dst: &str) -> Result<()> {
    let source = File::open(src)?;
    let destination = File::create(dst)?;

    let mut reader = BufReader::new(source);
    let mut writer = BufWriter::new(destination);

    io::copy(&mut reader, &mut writer)?;

    writer.flush()?; // Ensures all buffered contents are written to the file

    Ok(())
}

/// Read an SSH public key file and return a String.
pub fn get_ssh_public_key(path: &str) -> Result<String> {
    let full_path = expand_path(path);
    let file = File::open(&full_path)?;
    let reader = BufReader::new(file);

    // Read the first line of the file
    if let Some(line) = reader.lines().next() {
        let key = line?;
        Ok(key)
    } else {
        Err(anyhow::anyhow!("Invalid SSH public key file: {full_path}",))
    }
}

/// Set permissions for all files in a folder subtree.
/// Sets files to read-only and removes executable bit for users/groups.
pub fn fix_permissions_recursive(path: &str) -> Result<()> {
    let path = expand_path(path);

    let metadata = fs::metadata(&path)?;
    let mut perms = metadata.permissions();

    if metadata.is_dir() {
        // Set directory permissions to 0755
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;

        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            fix_permissions_recursive(
                entry
                    .path()
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Error updating read-only permissions"))?,
            )?;
        }
    } else {
        // Set file permissions to 0444
        perms.set_mode(0o440);
        fs::set_permissions(path, perms)?;
    }

    Ok(())
}
