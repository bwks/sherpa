use anyhow::Result;

use shared::konst::SHERPA_IMAGES_PATH;
use shared::util::{fix_permissions_recursive, term_msg_surround};

pub fn doctor(boxes: bool) -> Result<()> {
    if boxes {
        term_msg_surround("Fixing base box permissions");
        fix_permissions_recursive(SHERPA_IMAGES_PATH)?;
    }
    Ok(())
}
