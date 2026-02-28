use anyhow::Result;
use serde_derive::{Deserialize, Serialize};

use shared::data::NodeModel;
use shared::util::split_node_int;

/// Manifest Link
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Link {
    pub node_a: String,
    pub int_a: u8,
    pub node_b: String,
    pub int_b: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LinkExpanded {
    pub node_a: String,
    pub int_a: String,
    pub node_b: String,
    pub int_b: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LinkDetailed {
    pub node_a: String,
    pub node_a_idx: u16,
    pub node_a_model: NodeModel,
    pub int_a: String,
    pub int_a_idx: u8,
    pub node_b: String,
    pub node_b_idx: u16,
    pub node_b_model: NodeModel,
    pub int_b: String,
    pub int_b_idx: u8,
    pub link_idx: u16,
}

/// Manifest Link
/// expected format:
/// "node_name::int_name" - Seperated by double collon `::`
/// {src = "<src_node_name>::<src_int_name>", dst = "<dst_node_name>::<dst_int_name>" }
/// {src = "dev01::eth1", dst = "dev02::eth0" }
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Link2 {
    pub src: String,
    pub dst: String,
    pub p2p: Option<bool>,
}

impl Link2 {
    pub fn expand(&self) -> Result<LinkExpanded> {
        let (node_a, int_a) = split_node_int(&self.src)?;
        let (node_b, int_b) = split_node_int(&self.dst)?;

        Ok(LinkExpanded {
            node_a,
            int_a,
            node_b,
            int_b,
        })
    }
}
