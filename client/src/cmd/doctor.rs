use anyhow::Result;

use util::{fix_permissions_recursive, term_msg_surround};

pub fn doctor(boxes: bool, boxes_dir: &str) -> Result<()> {
    if boxes {
        term_msg_surround("Fixing base box permissions");

        fix_permissions_recursive(boxes_dir)?;
    }
    Ok(())
}
