use anyhow::Result;
use virt::connect::Connect;

use crate::core::konst::QEMU_URI;

pub struct Qemu {
    pub uri: String,
}

impl Default for Qemu {
    fn default() -> Self {
        Self {
            uri: QEMU_URI.to_owned(),
        }
    }
}

impl Qemu {
    pub fn connect(&self) -> Result<Connect> {
        let conn = Connect::open(Some(self.uri.as_str()))?;
        // println!("Connected to hypervisor: {}", self.uri);
        Ok(conn)
    }
}
