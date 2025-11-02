mod arista_eos;
mod aruba_aos;
mod cisco_asa;
mod cisco_ios;
mod cisco_iosxe;
mod cisco_iosxr;
mod cisco_nxos;
mod cloud_init;
mod cumulus_linux;
mod domain;
mod ignition;
mod juniper_junos;
mod pyats;
mod ssh;

pub use crate::template::arista_eos::{AristaVeosZtpTemplate, arista_veos_ztp_script};
pub use crate::template::aruba_aos::{ArubaAoscxShTemplate, ArubaAoscxTemplate};
pub use crate::template::cisco_asa::CiscoAsavZtpTemplate;
pub use crate::template::cisco_ios::{CiscoIosvZtpTemplate, CiscoIosvl2ZtpTemplate};
pub use crate::template::cisco_iosxe::CiscoIosXeZtpTemplate;
pub use crate::template::cisco_iosxr::CiscoIosxrZtpTemplate;
pub use crate::template::cisco_nxos::CiscoNxosZtpTemplate;
pub use crate::template::cloud_init::{CloudInitConfig, CloudInitUser};
pub use crate::template::cumulus_linux::CumulusLinuxZtpTemplate;
pub use crate::template::ignition::{
    Contents, File, FileParams, FileSystem, IgnitionConfig, Link, Unit, User,
};
pub use crate::template::juniper_junos::{JunipervJunosZtpTemplate, juniper_vevolved_ztp_script};
pub use crate::template::pyats::PyatsInventory;
pub use crate::template::ssh::SshConfigTemplate;
