pub mod arista_eos;
pub mod aruba_aos;
pub mod cisco_asa;
pub mod cisco_ios;
pub mod cisco_iosxe;
pub mod cisco_iosxr;
pub mod cisco_nxos;
pub mod cumulus_linux;

pub use crate::bootstrap::arista_eos::{arista_veos_ztp_script, AristaVeosZtpTemplate};
pub use crate::bootstrap::aruba_aos::ArubaAoscxTemplate;
pub use crate::bootstrap::cisco_asa::CiscoAsavZtpTemplate;
pub use crate::bootstrap::cisco_ios::CiscoIosvZtpTemplate;
pub use crate::bootstrap::cisco_iosxe::CiscoIosXeZtpTemplate;
pub use crate::bootstrap::cisco_iosxr::CiscoIosxrZtpTemplate;
pub use crate::bootstrap::cisco_nxos::CiscoNxosZtpTemplate;
pub use crate::bootstrap::cumulus_linux::CumulusLinuxZtpTemplate;
