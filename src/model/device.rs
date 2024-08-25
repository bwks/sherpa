use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceModels {
    CiscoCsr1000v,
    CiscoCat8000v,
    CiscoCat9000v,
    CiscoIosxrv9000,
    CiscoNexus9300v,
    CiscoIosv,
    CiscoIosvl2,
    AristaVeos,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Manufacturers {
    Cisco,
    Arista,
    Juniper,
    Nvidia,
    Nokia,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OsVariants {
    Ios,
    Iosxe,
    Iosxr,
    Nxos,
    Junos,
    Cumulus,
    Sros,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceModel {
    pub name: DeviceModels,
    pub os_variant: OsVariants,
    pub manufacturer: Manufacturers,
    pub num_interfaces: u8,
    pub int_prefix: String,
    pub cpus: u8,
    pub memory: u16,
}
impl DeviceModel {
    pub fn cisco_csr1000v() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoCsr1000v,
            os_variant: OsVariants::Iosxe,
            manufacturer: Manufacturers::Cisco,
            num_interfaces: 8,
            int_prefix: "Gig0/".to_owned(),
            cpus: 2,
            memory: 4096,
        }
    }
    pub fn cisco_cat9000v() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoCat9000v,
            os_variant: OsVariants::Iosxe,
            manufacturer: Manufacturers::Cisco,
            num_interfaces: 8,
            int_prefix: "Gig0/0/".to_owned(),
            cpus: 4,
            memory: 16384,
        }
    }
    pub fn cisco_iosv() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoIosv,
            os_variant: OsVariants::Ios,
            manufacturer: Manufacturers::Cisco,
            num_interfaces: 8,
            int_prefix: "Gig".to_owned(),
            cpus: 1,
            memory: 1024,
        }
    }
    pub fn cisco_iosxrv9000() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoIosxrv9000,
            os_variant: OsVariants::Iosxr,
            manufacturer: Manufacturers::Cisco,
            num_interfaces: 8,
            int_prefix: "Gig0/0/0/".to_owned(),
            cpus: 4,
            memory: 16384,
        }
    }
}
