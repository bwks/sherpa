use anyhow::Result;

use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::file_exists;
use topology::Manifest;

pub fn new(force: bool) -> Result<()> {
    if file_exists(SHERPA_MANIFEST_FILE) && !force {
        println!(
            "{} already exists. Use --force to overwrite.",
            SHERPA_MANIFEST_FILE
        );
        return Ok(());
    }

    let manifest = Manifest::example()?;
    manifest.write_file(SHERPA_MANIFEST_FILE)?;

    println!("Created {} with example lab manifest.", SHERPA_MANIFEST_FILE);

    Ok(())
}
