use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceModels {
    CiscoCat9kv,
    CiscoIosv,
    AristaEos,
    Error(String),
}
// impl DeviceModels {
//     pub fn from_str(model: &str) -> DeviceModels {
//         match model {
//             "cisco_cat9kv" => DeviceModels::CiscoCat9kv,
//             "cisco_iosv" => DeviceModels::CiscoIosv,
//             "arista_eos" => DeviceModels::AristaEos,
//             other => DeviceModels::Error(other.to_string()),
//         }
//     }
// }

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
pub enum OsTypes {
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
    pub os_type: OsTypes,
    pub manufacturer: Manufacturers,
    pub num_interfaces: u8,
    pub int_prefix: String,
}
impl DeviceModel {
    pub fn cisco_cat9kv() -> DeviceModel {
        DeviceModel {
            name: DeviceModels::CiscoCat9kv,
            os_type: OsTypes::Iosxe,
            manufacturer: Manufacturers::Cisco,
            num_interfaces: 8,
            int_prefix: "Gig0/0/".to_owned(),
        }
    }
}
