use anyhow::Result;

use shared::konst::{SHERPA_BASE_DIR, SHERPA_IMAGES_DIR};
use shared::util::{fix_permissions_recursive, term_msg_surround};

pub fn doctor(boxes: bool) -> Result<()> {
    if boxes {
        term_msg_surround("Fixing base box permissions");
        let images_dir = format!("{SHERPA_BASE_DIR}/{SHERPA_IMAGES_DIR}");
        fix_permissions_recursive(&images_dir)?;
    }
    Ok(())
}
