use std::path::Path;

use anyhow::Result;

use shared::konst::SHERPA_MANIFEST_FILE;
use shared::util::{get_id, term_msg_surround};
use topology::Manifest;

use crate::provision::provision_lab;

/// Run the lab's node provisioners again, from the lab folder.
pub fn provision(node: Option<&str>) -> Result<()> {
    let manifest = Manifest::load_file(SHERPA_MANIFEST_FILE)?;
    let lab_id = get_id(&manifest.name)?;
    term_msg_surround(&format!("Provision Lab - {}-{lab_id}", manifest.name));

    let manifest_dir = Path::new(SHERPA_MANIFEST_FILE)
        .parent()
        .unwrap_or(Path::new("."));
    let provisioned = provision_lab(&manifest, manifest_dir, &lab_id, None, node)?;
    if provisioned == 0 {
        println!("No provisioners to run");
    }
    Ok(())
}
