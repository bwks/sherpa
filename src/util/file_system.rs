use std::fs;
use std::path::Path;

use anyhow::Result;

/// Create a directory, expanding ~ if it's passed
pub fn create_dir(name: &str) -> Result<()> {
    let expanded_path = shellexpand::tilde(name);
    let path = Path::new(expanded_path.as_ref());
    if !path.exists() {
        fs::create_dir_all(path)?;
        println!("Directory path created successfully: {name}");
    } else {
        println!("Directory path already exists: {name}");
    }

    Ok(())
}

/// Create a file, expanding ~ if it's passed
pub fn create_file(file_path: &str, contents: String) -> Result<()> {
    let expanded_path = shellexpand::tilde(file_path);
    let path = Path::new(expanded_path.as_ref());

    fs::write(path, contents)?;

    Ok(())
}

pub fn file_exists(file_path: &str) -> bool {
    let expanded_path = shellexpand::tilde(file_path);
    let path = Path::new(expanded_path.as_ref());
    path.exists() && path.is_file()
}

pub fn dir_exists(dir_path: &str) -> bool {
    let expanded_path = shellexpand::tilde(dir_path);
    let path = Path::new(expanded_path.as_ref());
    path.exists() && path.is_dir()
}
