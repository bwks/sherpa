pub mod arista_veos;
pub mod cisco_ios;
pub mod cisco_iosxe;
pub mod cumulus_linux;

pub use crate::bootstrap::arista_veos::{arista_veos_ztp_script, AristaVeosZtpTemplate};
pub use crate::bootstrap::cisco_ios::CiscoIosvZtpTemplate;
pub use crate::bootstrap::cisco_iosxe::CiscoIosXeZtpTemplate;
pub use crate::bootstrap::cumulus_linux::CumulusLinuxZtpTemplate;
