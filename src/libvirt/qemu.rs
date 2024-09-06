use anyhow::Result;
use virt::connect::Connect;

pub struct Qemu {
    pub uri: String,
}

impl Default for Qemu {
    fn default() -> Self {
        Self {
            uri: "qemu:///system".to_owned(),
        }
    }
}

impl Qemu {
    pub fn connect(&self) -> Result<Connect> {
        let conn = Connect::open(Some(self.uri.as_str()))?;
        println!("Connected to hypervisor: {:?}", conn);
        Ok(conn)
    }
}

pub struct QemuImage {
    pub name: String,
}

impl QemuImage {
    pub fn clone(self) {}
}
