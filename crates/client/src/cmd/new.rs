use anyhow::{Result, bail};

use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::file_exists;
use topology::Manifest;

/// Write an example `manifest.toml`, refusing to replace one unless `force` is set.
pub fn new(force: bool) -> Result<()> {
    if file_exists(SHERPA_MANIFEST_FILE) && !force {
        bail!("{SHERPA_MANIFEST_FILE} already exists. Use --force to overwrite.");
    }

    let manifest = Manifest::example()?;
    manifest.write_file(SHERPA_MANIFEST_FILE)?;

    println!("Created {SHERPA_MANIFEST_FILE} with example lab manifest.");

    Ok(())
}
