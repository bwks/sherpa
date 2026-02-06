mod arista_eos;
mod aruba_aos;
mod cisco_asa;
mod cisco_ftdv;
mod cisco_ios;
mod cisco_iosxe;
mod cisco_iosxr;
mod cisco_ise;
mod cisco_nxos;
mod cloud_init;
mod cumulus_linux;
mod dnsmasq;
mod domain;
mod ignition;
mod juniper_junos;
mod pyats;
mod sonic_linux;
mod ssh;

pub use arista_eos::{AristaCeosZtpTemplate, AristaVeosZtpTemplate};
pub use aruba_aos::ArubaAoscxTemplate;
pub use cisco_asa::CiscoAsavZtpTemplate;
pub use cisco_ftdv::{CiscoFtdvZtpTemplate, CiscoFxosIpMode};
pub use cisco_ios::{CiscoIosvZtpTemplate, CiscoIosvl2ZtpTemplate};
pub use cisco_iosxe::CiscoIosXeZtpTemplate;
pub use cisco_iosxr::CiscoIosxrZtpTemplate;
pub use cisco_ise::CiscoIseZtpTemplate;
pub use cisco_nxos::CiscoNxosZtpTemplate;
pub use cloud_init::{
    CloudInitConfig, CloudInitNetwork, CloudInitResolvConf, CloudInitUser, MetaDataConfig,
};
pub use cumulus_linux::CumulusLinuxZtpTemplate;
pub use dnsmasq::DnsmasqTemplate;
pub use domain::{BootServer, DomainTemplate};
pub use ignition::{
    IgnitionConfig, IgnitionFile, IgnitionFileContents, IgnitionFileParams, IgnitionFileSystem,
    IgnitionLink, IgnitionUnit, IgnitionUser,
};
pub use juniper_junos::JunipervJunosZtpTemplate;
pub use pyats::PyatsInventory;
pub use sonic_linux::{SonicLinuxUserTemplate, SonicLinuxZtp};
pub use ssh::SshConfigTemplate;
