use serde_derive::{Deserialize, Serialize};

#[derive(Default, Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum VmProviders {
    #[default]
    Libvirt,
    // Future use maybe, if someone wants to add it.
}
