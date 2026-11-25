use thiserror::Error;

use crate::host::NumaCellValue;

/// A libvirt operation's failure that a caller tells apart from the others.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LibvirtError {
    #[error("The server's capabilities are not valid XML")]
    CapabilitiesNotXml,

    #[error("A NUMA cell in the capabilities has no id")]
    NumaCellWithoutId,

    #[error("Cannot read NUMA node {node}'s {value}")]
    NumaCellUnreadable { node: u32, value: NumaCellValue },

    #[error("The {value} has no value")]
    NumaCellValueMissing { value: NumaCellValue },

    #[error("Unknown memory unit '{unit}'")]
    UnknownMemoryUnit { unit: String },

    #[error("Cannot parse the domain XML")]
    DomainXmlInvalid,

    #[error("Cannot parse the domain XML at byte {position}")]
    DomainXmlInvalidAt { position: u64 },

    #[error("Cannot parse the domain XML: it ends inside an element")]
    DomainXmlEndsInsideElement,
}
