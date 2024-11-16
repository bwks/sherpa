pub mod arista_veos;
pub mod cumulus_linux;

pub use crate::bootstrap::arista_veos::{arista_veos_ztp_script, AristaVeosZtpTemplate};
pub use crate::bootstrap::cumulus_linux::CumulusLinuxZtpTemplate;
