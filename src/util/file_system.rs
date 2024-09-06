use std::fs;
use std::path::Path;

use anyhow::Result;

/// Create a directory, expanding ~ if it's passed
pub fn create_dir(name: &str) -> Result<()> {
    let expanded_path = shellexpand::tilde(name);
    let path = Path::new(expanded_path.as_ref());

    fs::create_dir_all(path)?;
    println!("Directory created successfully: {:?}", path);

    Ok(())
}
