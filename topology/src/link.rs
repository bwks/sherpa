use serde_derive::{Deserialize, Serialize};

/// Manifest Link
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Link {
    pub dev_a: String,
    pub int_a: u8,
    pub dev_b: String,
    pub int_b: u8,
}
