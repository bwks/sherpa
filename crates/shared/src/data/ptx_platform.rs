use std::fmt;

use serde_derive::{Deserialize, Serialize};

/// The PTX router a `juniper_cevolved` node emulates: `platform` on a node in the manifest.
///
/// cJunosEvolved simulates the forwarding chipset of one of two fixed PTX routers.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PtxPlatform {
    /// PTX10001-36MR, with the Express 4 (BT) chipset.
    #[default]
    Bt,
    /// PTX10002-36QDD, with the Express 5 (BX) chipset.
    Bx,
}

impl PtxPlatform {
    /// The value of the container's `CPTX_COSIM` variable, which selects the platform.
    #[must_use]
    pub fn cosim(self) -> &'static str {
        match self {
            PtxPlatform::Bt => "BT",
            PtxPlatform::Bx => "BX",
        }
    }

    /// The most data interfaces the platform has, unchannelized: `et-0/0/0` onwards.
    #[must_use]
    pub fn max_data_interfaces(self) -> u8 {
        match self {
            PtxPlatform::Bt => 12,
            PtxPlatform::Bx => 36,
        }
    }
}

impl fmt::Display for PtxPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PtxPlatform::Bt => write!(f, "bt"),
            PtxPlatform::Bx => write!(f, "bx"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_names_are_lowercase() {
        for platform in [PtxPlatform::Bt, PtxPlatform::Bx] {
            let json = serde_json::to_value(platform).unwrap();
            assert_eq!(json.as_str().unwrap(), platform.to_string());
        }
        assert!(serde_json::from_str::<PtxPlatform>(r#""BX""#).is_err());
        assert!(serde_json::from_str::<PtxPlatform>(r#""bz""#).is_err());
    }

    #[test]
    fn bt_is_the_default_and_has_12_ports() {
        assert_eq!(PtxPlatform::default(), PtxPlatform::Bt);
        assert_eq!(PtxPlatform::Bt.cosim(), "BT");
        assert_eq!(PtxPlatform::Bt.max_data_interfaces(), 12);
        assert_eq!(PtxPlatform::Bx.cosim(), "BX");
        assert_eq!(PtxPlatform::Bx.max_data_interfaces(), 36);
    }
}
