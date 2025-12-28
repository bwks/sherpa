use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use data::NodeModel;
use util::split_dev_int;

/// Manifest Link
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Link {
    pub dev_a: String,
    pub int_a: u8,
    pub dev_b: String,
    pub int_b: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LinkExpanded {
    pub dev_a: String,
    pub int_a: String,
    pub dev_b: String,
    pub int_b: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LinkDetailed {
    pub dev_a: String,
    pub dev_a_model: NodeModel,
    pub int_a: String,
    pub int_a_idx: u8,
    pub dev_b: String,
    pub dev_b_model: NodeModel,
    pub int_b: String,
    pub int_b_idx: u8,
}

/// Manifest Link
/// expected format:
/// "dev_name::int_name" - Seperated by double collon `::`
/// {src = "<src_dev_name>::<src_int_name>", dst = "<dst_dev_name>::<dst_int_name>" }
/// {src = "dev01::eth1", dst = "dev02::eth0" }
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Link2 {
    pub src: String,
    pub dst: String,
}

impl Link2 {
    pub fn expand(&self) -> Result<LinkExpanded> {
        let (dev_a, int_a) = split_dev_int(&self.src)?;
        let (dev_b, int_b) = split_dev_int(&self.dst)?;

        Ok(LinkExpanded {
            dev_a,
            int_a,
            dev_b,
            int_b,
        })
    }
}
