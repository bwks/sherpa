use std::fs;
use std::path::Path;

use anyhow::Result;

/// Create a directory, expanding ~ if it's passed
pub fn create_dir(dir_path: &str) -> Result<()> {
    fs::create_dir_all(&dir_path)?;
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

/// Copy a file
pub fn copy_file(source: &str, destination: &str) -> Result<()> {
    fs::copy(source, destination)?;
    println!("File copied successfully: {destination}");
    Ok(())
}
