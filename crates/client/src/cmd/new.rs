use anyhow::Result;

use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::file_exists;
use topology::Manifest;

pub fn new(force: bool) -> Result<()> {
    if file_exists(SHERPA_MANIFEST_FILE) && !force {
        println!("{SHERPA_MANIFEST_FILE} already exists. Use --force to overwrite.");
        return Ok(());
    }

    let manifest = Manifest::example()?;
    manifest.write_file(SHERPA_MANIFEST_FILE)?;

    println!("Created {SHERPA_MANIFEST_FILE} with example lab manifest.");

    Ok(())
}
