use std::fmt;

use serde_derive::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use thiserror::Error;

use super::mapping::InterfaceConnection;

pub trait InterfaceTrait {
    fn to_idx(&self) -> u8;
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError>;
}

#[derive(Debug, Error)]
pub enum ParseInterfaceStrError {
    #[error("Unknown interface for {enum_name}: {iface}")]
    UnknownInterfaceStr {
        enum_name: &'static str,
        iface: String,
    },
}
#[derive(Debug, Error)]
pub enum ParseInterfaceIdxError {
    #[error("Unknown interface index for {enum_name}: {idx}")]
    UnknownInterfaceIdx { enum_name: &'static str, idx: u8 },
}

macro_rules! interface_enum {
    (
        $enum_name:ident,
        $enum_str:expr,
        [ $( $variant:ident => $idx:expr, $name:expr ),* ]
    ) => {
        #[derive(Serialize, Deserialize, Clone, Debug)]
        pub enum $enum_name {
            $(
                #[serde(rename = $name)]
                $variant,
            )*
        }
        impl $enum_name {
            /// Returns a vector of all interface names for this enum
            pub fn all_interfaces() -> Vec<String> {
                vec![
                    $(
                        $name.to_string(),
                    )*
                ]
            }
        }

        impl InterfaceTrait for $enum_name {
            fn to_idx(&self) -> u8 {
                match self {
                    $(
                        $enum_name::$variant => $idx,
                    )*
                }
            }
            fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
                let iface = match idx {
                    $(
                        $idx => $name,
                    )*
                    _ => "",
                };
                if iface.is_empty() {
                    Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                        enum_name: $enum_str,
                        idx,
                    })
                } else {
                    Ok(iface.to_string())
                }
            }
        }

        impl std::str::FromStr for $enum_name {
            type Err = ParseInterfaceStrError;
            fn from_str(text: &str) -> Result<Self, Self::Err> {
                match text {
                    $(
                        $name => Ok($enum_name::$variant),
                    )*
                    _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                        enum_name: $enum_str,
                        iface: text.to_string(),
                    }),
                }
            }
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum InterfaceTypes {
    #[default]
    Unknown,
    Mgmt,
    Eth,
    Swp,
    Gig,
    Ten,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, EnumIter)]
#[serde(rename_all = "lowercase")]
pub enum MgmtInterfaces {
    #[default]
    #[serde(rename = "eth0")]
    Eth0, // eth0 - cumulus-vx, linux
    #[serde(rename = "GigabitEthernet0/0")]
    GigabitEthernet0_0, // GigabitEthernet0/0 - cat9k, iosv/l2
    #[serde(rename = "GigabitEthernet1")]
    GigabitEthernet1, // GigabitEthernet1 - cat1/8k
    #[serde(rename = "re0:mgmt-0")]
    Re0Mgmt0, // fxp0 - Junos
    #[serde(rename = "fxp0")]
    Fxp0, // fxp0 - Junos
    #[serde(rename = "fxp0.0")]
    Fxp0_0, // fxp0.0 - Junos
    #[serde(rename = "mgmt")]
    Mgmt, // mgmt - aos
    #[serde(rename = "mgmt0")]
    Mgmt0, // mgmt0 - n93kv
    #[serde(rename = "Management0/0")]
    Management0_0, // Management0/0 - asav
    #[serde(rename = "Management1")]
    Management1, // Management1 - eos
    #[serde(rename = "MgmtEth0/RP0/CPU0/0")]
    MgmtEth0Rp0Cpu0_0, // MgmtEth0/RP0/CPU0/0 - xr9kv
}
impl fmt::Display for MgmtInterfaces {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MgmtInterfaces::Eth0 => write!(f, "eth0"),
            MgmtInterfaces::GigabitEthernet1 => write!(f, "GigabitEthernet1"),
            MgmtInterfaces::GigabitEthernet0_0 => write!(f, "GigabitEthernet0/0"),
            MgmtInterfaces::Re0Mgmt0 => write!(f, "re0:mgmt-0"),
            MgmtInterfaces::Fxp0 => write!(f, "fxp0"),
            MgmtInterfaces::Fxp0_0 => write!(f, "fxp0.0"),
            MgmtInterfaces::Mgmt => write!(f, "mgmt"),
            MgmtInterfaces::Mgmt0 => write!(f, "mgmt0"),
            MgmtInterfaces::Management1 => write!(f, "Management1"),
            MgmtInterfaces::Management0_0 => write!(f, "Management0/0"),
            MgmtInterfaces::MgmtEth0Rp0Cpu0_0 => write!(f, "MgmtEth0/RP0/CPU0/0"),
        }
    }
}
impl MgmtInterfaces {
    pub fn to_vec() -> Vec<MgmtInterfaces> {
        MgmtInterfaces::iter().collect()
    }
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionTypes {
    #[default]
    Disabled, // Disable interface
    Management,    // Connects to management bridge
    Peer,          // Peered with another device
    PeerBridge,    // Peered with another device via a bridge
    PrivateBridge, // Attached to a private bridge
    Reserved,      // Reserved interfaces used by the virtual platform
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Interface {
    pub name: String,
    pub num: u8,
    pub mac_address: String,
    pub mtu: u16,
    pub connection_type: ConnectionTypes,
    pub interface_connection: Option<InterfaceConnection>,
}

interface_enum!(
    ArubaAoscxInt,
    "ArubaAoscx",
    [
        Mgmt => 0, "mgmt",
        E1_1_1 => 1, "1/1/1",
        E1_1_2 => 2, "1/1/2",
        E1_1_3 => 3, "1/1/3",
        E1_1_4 => 4, "1/1/4",
        E1_1_5 => 5, "1/1/5",
        E1_1_6 => 6, "1/1/6",
        E1_1_7 => 7, "1/1/7",
        E1_1_8 => 8, "1/1/8",
        E1_1_9 => 9, "1/1/9",
        E1_1_10 => 10, "1/1/10",
        E1_1_11 => 11, "1/1/11",
        E1_1_12 => 12, "1/1/12",
        E1_1_13 => 13, "1/1/13",
        E1_1_14 => 14, "1/1/14",
        E1_1_15 => 15, "1/1/15",
        E1_1_16 => 16, "1/1/16",
        E1_1_17 => 17, "1/1/17",
        E1_1_18 => 18, "1/1/18",
        E1_1_19 => 19, "1/1/19",
        E1_1_20 => 20, "1/1/20",
        E1_1_21 => 21, "1/1/21",
        E1_1_22 => 22, "1/1/22",
        E1_1_23 => 23, "1/1/23",
        E1_1_24 => 24, "1/1/24",
        E1_1_25 => 25, "1/1/25",
        E1_1_26 => 26, "1/1/26",
        E1_1_27 => 27, "1/1/27",
        E1_1_28 => 28, "1/1/28",
        E1_1_29 => 29, "1/1/29",
        E1_1_30 => 30, "1/1/30",
        E1_1_31 => 31, "1/1/31",
        E1_1_32 => 32, "1/1/32",
        E1_1_33 => 33, "1/1/33",
        E1_1_34 => 34, "1/1/34",
        E1_1_35 => 35, "1/1/35",
        E1_1_36 => 36, "1/1/36",
        E1_1_37 => 37, "1/1/37",
        E1_1_38 => 38, "1/1/38",
        E1_1_39 => 39, "1/1/39",
        E1_1_40 => 40, "1/1/40",
        E1_1_41 => 41, "1/1/41",
        E1_1_42 => 42, "1/1/42",
        E1_1_43 => 43, "1/1/43",
        E1_1_44 => 44, "1/1/44",
        E1_1_45 => 45, "1/1/45",
        E1_1_46 => 46, "1/1/46",
        E1_1_47 => 47, "1/1/47",
        E1_1_48 => 48, "1/1/48",
        E1_1_49 => 49, "1/1/49",
        E1_1_50 => 50, "1/1/50",
        E1_1_51 => 51, "1/1/51",
        E1_1_52 => 52, "1/1/52"
    ]
);

interface_enum!(
    EthernetInt,
    "EthernetInt",
    [
        Eth0 => 0, "eth0",
        Eth1 => 1, "eth1",
        Eth2 => 2, "eth2",
        Eth3 => 3, "eth3",
        Eth4 => 4, "eth4",
        Eth5 => 5, "eth5",
        Eth6 => 6, "eth6",
        Eth7 => 7, "eth7",
        Eth8 => 8, "eth8",
        Eth9 => 9, "eth9",
        Eth10 => 10, "eth10",
        Eth11 => 11, "eth11",
        Eth12 => 12, "eth12",
        Eth13 => 13, "eth13",
        Eth14 => 14, "eth14",
        Eth15 => 15, "eth15",
        Eth16 => 16, "eth16",
        Eth17 => 17, "eth17",
        Eth18 => 18, "eth18",
        Eth19 => 19, "eth19",
        Eth20 => 20, "eth20",
        Eth21 => 21, "eth21",
        Eth22 => 22, "eth22",
        Eth23 => 23, "eth23",
        Eth24 => 24, "eth24",
        Eth25 => 25, "eth25",
        Eth26 => 26, "eth26",
        Eth27 => 27, "eth27",
        Eth28 => 28, "eth28",
        Eth29 => 29, "eth29",
        Eth30 => 30, "eth30",
        Eth31 => 31, "eth31",
        Eth32 => 32, "eth32",
        Eth33 => 33, "eth33",
        Eth34 => 34, "eth34",
        Eth35 => 35, "eth35",
        Eth36 => 36, "eth36",
        Eth37 => 37, "eth37",
        Eth38 => 38, "eth38",
        Eth39 => 39, "eth39",
        Eth40 => 40, "eth40",
        Eth41 => 41, "eth41",
        Eth42 => 42, "eth42",
        Eth43 => 43, "eth43",
        Eth44 => 44, "eth44",
        Eth45 => 45, "eth45",
        Eth46 => 46, "eth46",
        Eth47 => 47, "eth47",
        Eth48 => 48, "eth48",
        Eth49 => 49, "eth49",
        Eth50 => 50, "eth50",
        Eth51 => 51, "eth51",
        Eth52 => 52, "eth52"
    ]
);

interface_enum!(
    CumulusLinuxInt,
    "CumulusLinuxInt",
    [
        Eth0 => 0, "eth0",
        Swp1 => 1, "swp1",
        Swp2 => 2, "swp2",
        Swp3 => 3, "swp3",
        Swp4 => 4, "swp4",
        Swp5 => 5, "swp5",
        Swp6 => 6, "swp6",
        Swp7 => 7, "swp7",
        Swp8 => 8, "swp8",
        Swp9 => 9, "swp9",
        Swp10 => 10, "swp10",
        Swp11 => 11, "swp11",
        Swp12 => 12, "swp12",
        Swp13 => 13, "swp13",
        Swp14 => 14, "swp14",
        Swp15 => 15, "swp15",
        Swp16 => 16, "swp16",
        Swp17 => 17, "swp17",
        Swp18 => 18, "swp18",
        Swp19 => 19, "swp19",
        Swp20 => 20, "swp20",
        Swp21 => 21, "swp21",
        Swp22 => 22, "swp22",
        Swp23 => 23, "swp23",
        Swp24 => 24, "swp24",
        Swp25 => 25, "swp25",
        Swp26 => 26, "swp26",
        Swp27 => 27, "swp27",
        Swp28 => 28, "swp28",
        Swp29 => 29, "swp29",
        Swp30 => 30, "swp30",
        Swp31 => 31, "swp31",
        Swp32 => 32, "swp32",
        Swp33 => 33, "swp33",
        Swp34 => 34, "swp34",
        Swp35 => 35, "swp35",
        Swp36 => 36, "swp36",
        Swp37 => 37, "swp37",
        Swp38 => 38, "swp38",
        Swp39 => 39, "swp39",
        Swp40 => 40, "swp40",
        Swp41 => 41, "swp41",
        Swp42 => 42, "swp42",
        Swp43 => 43, "swp43",
        Swp44 => 44, "swp44",
        Swp45 => 45, "swp45",
        Swp46 => 46, "swp46",
        Swp47 => 47, "swp47",
        Swp48 => 48, "swp48",
        Swp49 => 49, "swp49",
        Swp50 => 50, "swp50",
        Swp51 => 51, "swp51",
        Swp52 => 52, "swp52"
    ]
);

interface_enum!(
    AristaVeosInt,
    "AristaVeos",
    [
        Management1 => 0, "Management1",
        Eth1 => 1, "eth1",
        Eth2 => 2, "eth2",
        Eth3 => 3, "eth3",
        Eth4 => 4, "eth4",
        Eth5 => 5, "eth5",
        Eth6 => 6, "eth6",
        Eth7 => 7, "eth7",
        Eth8 => 8, "eth8",
        Eth9 => 9, "eth9",
        Eth10 => 10, "eth10",
        Eth11 => 11, "eth11",
        Eth12 => 12, "eth12",
        Eth13 => 13, "eth13",
        Eth14 => 14, "eth14",
        Eth15 => 15, "eth15",
        Eth16 => 16, "eth16",
        Eth17 => 17, "eth17",
        Eth18 => 18, "eth18",
        Eth19 => 19, "eth19",
        Eth20 => 20, "eth20",
        Eth21 => 21, "eth21",
        Eth22 => 22, "eth22",
        Eth23 => 23, "eth23",
        Eth24 => 24, "eth24",
        Eth25 => 25, "eth25",
        Eth26 => 26, "eth26",
        Eth27 => 27, "eth27",
        Eth28 => 28, "eth28",
        Eth29 => 29, "eth29",
        Eth30 => 30, "eth30",
        Eth31 => 31, "eth31",
        Eth32 => 32, "eth32",
        Eth33 => 33, "eth33",
        Eth34 => 34, "eth34",
        Eth35 => 35, "eth35",
        Eth36 => 36, "eth36",
        Eth37 => 37, "eth37",
        Eth38 => 38, "eth38",
        Eth39 => 39, "eth39",
        Eth40 => 40, "eth40",
        Eth41 => 41, "eth41",
        Eth42 => 42, "eth42",
        Eth43 => 43, "eth43",
        Eth44 => 44, "eth44",
        Eth45 => 45, "eth45",
        Eth46 => 46, "eth46",
        Eth47 => 47, "eth47",
        Eth48 => 48, "eth48",
        Eth49 => 49, "eth49",
        Eth50 => 50, "eth50",
        Eth51 => 51, "eth51",
        Eth52 => 52, "eth52"
    ]
);
interface_enum!(
    AristaCeosInt,
    "AristaCeos",
    [
        Management0 => 0, "Management0",
        Eth1 => 1, "eth1",
        Eth2 => 2, "eth2",
        Eth3 => 3, "eth3",
        Eth4 => 4, "eth4",
        Eth5 => 5, "eth5",
        Eth6 => 6, "eth6",
        Eth7 => 7, "eth7",
        Eth8 => 8, "eth8",
        Eth9 => 9, "eth9",
        Eth10 => 10, "eth10",
        Eth11 => 11, "eth11",
        Eth12 => 12, "eth12",
        Eth13 => 13, "eth13",
        Eth14 => 14, "eth14",
        Eth15 => 15, "eth15",
        Eth16 => 16, "eth16",
        Eth17 => 17, "eth17",
        Eth18 => 18, "eth18",
        Eth19 => 19, "eth19",
        Eth20 => 20, "eth20",
        Eth21 => 21, "eth21",
        Eth22 => 22, "eth22",
        Eth23 => 23, "eth23",
        Eth24 => 24, "eth24",
        Eth25 => 25, "eth25",
        Eth26 => 26, "eth26",
        Eth27 => 27, "eth27",
        Eth28 => 28, "eth28",
        Eth29 => 29, "eth29",
        Eth30 => 30, "eth30",
        Eth31 => 31, "eth31",
        Eth32 => 32, "eth32",
        Eth33 => 33, "eth33",
        Eth34 => 34, "eth34",
        Eth35 => 35, "eth35",
        Eth36 => 36, "eth36",
        Eth37 => 37, "eth37",
        Eth38 => 38, "eth38",
        Eth39 => 39, "eth39",
        Eth40 => 40, "eth40",
        Eth41 => 41, "eth41",
        Eth42 => 42, "eth42",
        Eth43 => 43, "eth43",
        Eth44 => 44, "eth44",
        Eth45 => 45, "eth45",
        Eth46 => 46, "eth46",
        Eth47 => 47, "eth47",
        Eth48 => 48, "eth48",
        Eth49 => 49, "eth49",
        Eth50 => 50, "eth50",
        Eth51 => 51, "eth51",
        Eth52 => 52, "eth52"
    ]
);

interface_enum!(
    CiscoIosvInt,
    "CiscoIosv",
    [
        Gig0_0 => 0, "gig0/0",
        Gig0_1 => 1, "gig0/1",
        Gig0_2 => 2, "gig0/2",
        Gig0_3 => 3, "gig0/3",
        Gig0_4 => 4, "gig0/4",
        Gig0_5 => 5, "gig0/5",
        Gig0_6 => 6, "gig0/6",
        Gig0_7 => 7, "gig0/7",
        Gig0_8 => 8, "gig0/8",
        Gig0_9 => 9, "gig0/9",
        Gig0_10 => 10, "gig0/10",
        Gig0_11 => 11, "gig0/11",
        Gig0_12 => 12, "gig0/12",
        Gig0_13 => 13, "gig0/13",
        Gig0_14 => 14, "gig0/14",
        Gig0_15 => 15, "gig0/15"
    ]
);

interface_enum!(
    CiscoIosvl2Int,
    "CiscoIosvl2",
    [
        Gig0_0 => 0, "gig0/0",
        Gig0_1 => 1, "gig0/1",
        Gig0_2 => 2, "gig0/2",
        Gig0_3 => 3, "gig0/3",
        Gig1_0 => 4, "gig1/0",
        Gig1_1 => 5, "gig1/1",
        Gig1_2 => 6, "gig1/2",
        Gig1_3 => 7, "gig1/3",
        Gig2_0 => 8, "gig2/0",
        Gig2_1 => 9, "gig2/1",
        Gig2_2 => 10, "gig2/2",
        Gig2_3 => 11, "gig2/3",
        Gig3_0 => 12, "gig3/0",
        Gig3_1 => 13, "gig3/1",
        Gig3_2 => 14, "gig3/2",
        Gig3_3 => 15, "gig3/3"
    ]
);

interface_enum!(
    CiscoIosxrv9000Int,
    "CiscoIosxrv9000Int",
    [
        MgmtEth0Rp0Cpu0_0 => 0, "MgmtEth0/RP0/CPU0/0",
        Reserved1 => 1, "reserved1",
        Reserved2 => 2, "reserved2",
        Gig0_0_0_0 => 3, "gig0/0/0/0",
        Gig0_0_0_1 => 4, "gig0/0/0/1",
        Gig0_0_0_2 => 5, "gig0/0/0/2",
        Gig0_0_0_3 => 6, "gig0/0/0/3",
        Gig0_0_0_4 => 7, "gig0/0/0/4",
        Gig0_0_0_5 => 8, "gig0/0/0/5",
        Gig0_0_0_6 => 9, "gig0/0/0/6",
        Gig0_0_0_7 => 10, "gig0/0/0/7",
        Gig0_0_0_8 => 11, "gig0/0/0/8",
        Gig0_0_0_9 => 12, "gig0/0/0/9",
        Gig0_0_0_10 => 13, "gig0/0/0/10",
        Gig0_0_0_11 => 14, "gig0/0/0/11",
        Gig0_0_0_12 => 15, "gig0/0/0/12",
        Gig0_0_0_13 => 16, "gig0/0/0/13",
        Gig0_0_0_14 => 17, "gig0/0/0/14",
        Gig0_0_0_15 => 18, "gig0/0/0/15",
        Gig0_0_0_16 => 19, "gig0/0/0/16",
        Gig0_0_0_17 => 20, "gig0/0/0/17",
        Gig0_0_0_18 => 21, "gig0/0/0/18",
        Gig0_0_0_19 => 22, "gig0/0/0/19",
        Gig0_0_0_20 => 23, "gig0/0/0/20",
        Gig0_0_0_21 => 24, "gig0/0/0/21",
        Gig0_0_0_22 => 25, "gig0/0/0/22",
        Gig0_0_0_23 => 26, "gig0/0/0/23",
        Gig0_0_0_24 => 27, "gig0/0/0/24",
        Gig0_0_0_25 => 28, "gig0/0/0/25",
        Gig0_0_0_26 => 29, "gig0/0/0/26",
        Gig0_0_0_27 => 30, "gig0/0/0/27",
        Gig0_0_0_28 => 31, "gig0/0/0/28"
    ]
);

interface_enum!(
    CiscoAsavInt,
    "CiscoAsav",
    [
        Mgmt0 => 0, "Management0/0",
        Gig0_0 => 1, "gig0/0",
        Gig0_1 => 2, "gig0/1",
        Gig0_2 => 3, "gig0/2",
        Gig0_3 => 4, "gig0/3",
        Gig0_4 => 5, "gig0/4",
        Gig0_5 => 6, "gig0/5",
        Gig0_6 => 7, "gig0/6",
        Gig0_7 => 8, "gig0/7"
    ]
);

interface_enum!(
    CiscoFtdvInt,
    "CiscoFtdv",
    [
        Management0_0 => 0, "Management0/0",
        Reserved1 => 1, "reserved1",
        Gig0_0 => 2, "gig0/0",
        Gig0_1 => 3, "gig0/1",
        Gig0_2 => 4, "gig0/2",
        Gig0_3 => 5, "gig0/3",
        Gig0_4 => 6, "gig0/4",
        Gig0_5 => 7, "gig0/5",
        Gig0_6 => 8, "gig0/6",
        Gig0_7 => 9, "gig0/7"
    ]
);

interface_enum!(
    CiscoCsr1000vInt,
    "CiscoCsr1000v",
    [
        Gig1 => 0, "gig1",
        Gig2 => 1, "gig2",
        Gig3 => 2, "gig3",
        Gig4 => 3, "gig4",
        Gig5 => 4, "gig5",
        Gig6 => 5, "gig6",
        Gig7 => 6, "gig7",
        Gig8 => 7, "gig8",
        Gig9 => 8, "gig9",
        Gig10 => 9, "gig10",
        Gig11 => 10, "gig11",
        Gig12 => 11, "gig12",
        Gig13 => 12, "gig13",
        Gig14 => 13, "gig14",
        Gig15 => 14, "gig15",
        Gig16 => 15, "gig16"
    ]
);
interface_enum!(
    CiscoCat8000vInt,
    "CiscoCat8000v",
    [
        Gig1 => 0, "gig1",
        Gig2 => 1, "gig2",
        Gig3 => 2, "gig3",
        Gig4 => 3, "gig4",
        Gig5 => 4, "gig5",
        Gig6 => 5, "gig6",
        Gig7 => 6, "gig7",
        Gig8 => 7, "gig8",
        Gig9 => 8, "gig9",
        Gig10 => 9, "gig10",
        Gig11 => 10, "gig11",
        Gig12 => 11, "gig12",
        Gig13 => 12, "gig13",
        Gig14 => 13, "gig14",
        Gig15 => 14, "gig15",
        Gig16 => 15, "gig16"
    ]
);

interface_enum!(
    CiscoCat9000vInt,
    "CiscoCat9000v",
    [
        Gig0_0_0 => 0, "gig0/0/0",
        Gig0_0_1 => 1, "gig0/0/1",
        Gig0_0_2 => 2, "gig0/0/2",
        Gig0_0_3 => 3, "gig0/0/3",
        Gig0_0_4 => 4, "gig0/0/4",
        Gig0_0_5 => 5, "gig0/0/5",
        Gig0_0_6 => 6, "gig0/0/6",
        Gig0_0_7 => 7, "gig0/0/7",
        Gig0_0_8 => 8, "gig0/0/8"
    ]
);

interface_enum!(
    CiscoNexus9300vInt,
    "CiscoNexus9300v",
    [
        Mgmt0 => 0, "mgmt0",
        Eth1_1 => 1, "eth1/1",
        Eth1_2 => 2, "eth1/2",
        Eth1_3 => 3, "eth1/3",
        Eth1_4 => 4, "eth1/4",
        Eth1_5 => 5, "eth1/5",
        Eth1_6 => 6, "eth1/6",
        Eth1_7 => 7, "eth1/7",
        Eth1_8 => 8, "eth1/8",
        Eth1_9 => 9, "eth1/9",
        Eth1_10 => 10, "eth1/10",
        Eth1_11 => 11, "eth1/11",
        Eth1_12 => 12, "eth1/12",
        Eth1_13 => 13, "eth1/13",
        Eth1_14 => 14, "eth1/14",
        Eth1_15 => 15, "eth1/15",
        Eth1_16 => 16, "eth1/16",
        Eth1_17 => 17, "eth1/17",
        Eth1_18 => 18, "eth1/18",
        Eth1_19 => 19, "eth1/19",
        Eth1_20 => 20, "eth1/20",
        Eth1_21 => 21, "eth1/21",
        Eth1_22 => 22, "eth1/22",
        Eth1_23 => 23, "eth1/23",
        Eth1_24 => 24, "eth1/24",
        Eth1_25 => 25, "eth1/25",
        Eth1_26 => 26, "eth1/26",
        Eth1_27 => 27, "eth1/27",
        Eth1_28 => 28, "eth1/28",
        Eth1_29 => 29, "eth1/29",
        Eth1_30 => 30, "eth1/30",
        Eth1_31 => 31, "eth1/31",
        Eth1_32 => 32, "eth1/32",
        Eth1_33 => 33, "eth1/33",
        Eth1_34 => 34, "eth1/34",
        Eth1_35 => 35, "eth1/35",
        Eth1_36 => 36, "eth1/36",
        Eth1_37 => 37, "eth1/37",
        Eth1_38 => 38, "eth1/38",
        Eth1_39 => 39, "eth1/39",
        Eth1_40 => 40, "eth1/40",
        Eth1_41 => 41, "eth1/41",
        Eth1_42 => 42, "eth1/42",
        Eth1_43 => 43, "eth1/43",
        Eth1_44 => 44, "eth1/44",
        Eth1_45 => 45, "eth1/45",
        Eth1_46 => 46, "eth1/46",
        Eth1_47 => 47, "eth1/47",
        Eth1_48 => 48, "eth1/48",
        Eth1_49 => 49, "eth1/49",
        Eth1_50 => 50, "eth1/50",
        Eth1_51 => 51, "eth1/51",
        Eth1_52 => 52, "eth1/52",
        Eth1_53 => 53, "eth1/53",
        Eth1_54 => 54, "eth1/54",
        Eth1_55 => 55, "eth1/55",
        Eth1_56 => 56, "eth1/56",
        Eth1_57 => 57, "eth1/57",
        Eth1_58 => 58, "eth1/58",
        Eth1_59 => 59, "eth1/59",
        Eth1_60 => 60, "eth1/60",
        Eth1_61 => 61, "eth1/61",
        Eth1_62 => 62, "eth1/62",
        Eth1_63 => 63, "eth1/63",
        Eth1_64 => 64, "eth1/64"
    ]
);
interface_enum!(
    JuniperVrouterInt,
    "JuniperVrouter",
    [
        Fxp0 => 0, "fxp0",
        Ge0_0_0 => 1, "ge-0/0/0",
        Ge0_0_1 => 2, "ge-0/0/1",
        Ge0_0_2 => 3, "ge-0/0/2",
        Ge0_0_3 => 4, "ge-0/0/3",
        Ge0_0_4 => 5, "ge-0/0/4",
        Ge0_0_5 => 6, "ge-0/0/5",
        Ge0_0_6 => 7, "ge-0/0/6",
        Ge0_0_7 => 8, "ge-0/0/7",
        Ge0_0_8 => 9, "ge-0/0/8",
        Ge0_0_9 => 10, "ge-0/0/9"
    ]
);

interface_enum!(
    JuniperVswitchInt,
    "JuniperVswitch",
    [
        Fxp0 => 0, "fxp0",
        Ge0_0_0 => 1, "ge-0/0/0",
        Ge0_0_1 => 2, "ge-0/0/1",
        Ge0_0_2 => 3, "ge-0/0/2",
        Ge0_0_3 => 4, "ge-0/0/3",
        Ge0_0_4 => 5, "ge-0/0/4",
        Ge0_0_5 => 6, "ge-0/0/5",
        Ge0_0_6 => 7, "ge-0/0/6",
        Ge0_0_7 => 8, "ge-0/0/7",
        Ge0_0_8 => 9, "ge-0/0/8",
        Ge0_0_9 => 10, "ge-0/0/9"
    ]
);

interface_enum!(
    JuniperVevolvedInt,
    "JuniperVevolved",
    [
        Re0Mgmt0 => 0, "re0:mgmt-0",
        Et0_0_0 => 1, "et-0/0/0",
        Et0_0_1 => 2, "et-0/0/1",
        Et0_0_2 => 3, "et-0/0/2",
        Et0_0_3 => 4, "et-0/0/3",
        Et0_0_4 => 5, "et-0/0/4",
        Et0_0_5 => 6, "et-0/0/5",
        Et0_0_6 => 7, "et-0/0/6",
        Et0_0_7 => 8, "et-0/0/7",
        Et0_0_8 => 9, "et-0/0/8",
        Et0_0_9 => 10, "et-0/0/9",
        Et0_0_10 => 11, "et-0/0/10",
        Et0_0_11 => 12, "et-0/0/11"
    ]
);

interface_enum!(
    JuniperVsrxv3Int,
    "JuniperVsrxv3",
    [
        Fxp0 => 0, "fxp0",
        Ge0_0_0 => 1, "ge-0/0/0",
        Ge0_0_1 => 2, "ge-0/0/1",
        Ge0_0_2 => 3, "ge-0/0/2",
        Ge0_0_3 => 4, "ge-0/0/3",
        Ge0_0_4 => 5, "ge-0/0/4",
        Ge0_0_5 => 6, "ge-0/0/5",
        Ge0_0_6 => 7, "ge-0/0/6",
        Ge0_0_7 => 8, "ge-0/0/7"
    ]
);

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use serde_test::{Token, assert_tokens};

//     #[test]
//     fn test_mgmt_interfaces_serialization() {
//         // Test Eth0 variant
//         assert_tokens(
//             &MgmtInterfaces::Eth0,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "eth0",
//             }],
//         );

//         // Test Mgmt variant
//         assert_tokens(
//             &MgmtInterfaces::Mgmt,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "mgmt",
//             }],
//         );
//         // Test Mgmt0 variant
//         assert_tokens(
//             &MgmtInterfaces::Mgmt0,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "mgmt0",
//             }],
//         );
//         // Test Management1 variant
//         assert_tokens(
//             &MgmtInterfaces::Management1,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "Management1",
//             }],
//         );
//         // Test Management0/0 variant
//         assert_tokens(
//             &MgmtInterfaces::Management0_0,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "Management0/0",
//             }],
//         );
//         // Test GigabitEthernet1 variant
//         assert_tokens(
//             &MgmtInterfaces::GigabitEthernet1,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "GigabitEthernet1",
//             }],
//         );
//         // Test GigabitEthernet0/0 variant
//         assert_tokens(
//             &MgmtInterfaces::GigabitEthernet0_0,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "GigabitEthernet0/0",
//             }],
//         );
//         // Test MgmtEth0/RP0/CPU0/0 variant
//         assert_tokens(
//             &MgmtInterfaces::MgmtEth0Rp0Cpu0_0,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "MgmtEth0/RP0/CPU0/0",
//             }],
//         );
//         // Test re0:mgmt-0 variant
//         assert_tokens(
//             &MgmtInterfaces::Re0Mgmt0,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "re0:mgmt-0",
//             }],
//         );
//         // Test fxp0 variant
//         assert_tokens(
//             &MgmtInterfaces::Fxp0,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "fxp0",
//             }],
//         );
//         // Test fxp0.0 variant
//         assert_tokens(
//             &MgmtInterfaces::Fxp0_0,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "fxp0.0",
//             }],
//         );
//         // Test Vlan1 variant
//         assert_tokens(
//             &MgmtInterfaces::Vlan1,
//             &[Token::UnitVariant {
//                 name: "MgmtInterfaces",
//                 variant: "Vlan1",
//             }],
//         );
//     }

//     #[test]
//     fn test_mgmt_interfaces_deserialization() {
//         // Test string to enum conversion
//         let eth0: MgmtInterfaces = serde_json::from_str(r#""eth0""#).unwrap();
//         assert!(matches!(eth0, MgmtInterfaces::Eth0));

//         let mgmt: MgmtInterfaces = serde_json::from_str(r#""mgmt""#).unwrap();
//         assert!(matches!(mgmt, MgmtInterfaces::Mgmt));

//         let mgmt0: MgmtInterfaces = serde_json::from_str(r#""mgmt0""#).unwrap();
//         assert!(matches!(mgmt0, MgmtInterfaces::Mgmt0));

//         let management1: MgmtInterfaces = serde_json::from_str(r#""Management1""#).unwrap();
//         assert!(matches!(management1, MgmtInterfaces::Management1));

//         let management0_0: MgmtInterfaces = serde_json::from_str(r#""Management0/0""#).unwrap();
//         assert!(matches!(management0_0, MgmtInterfaces::Management0_0));

//         let gigabit_ethernet1: MgmtInterfaces =
//             serde_json::from_str(r#""GigabitEthernet1""#).unwrap();
//         assert!(matches!(
//             gigabit_ethernet1,
//             MgmtInterfaces::GigabitEthernet1
//         ));

//         let gigabit_ethernet0_0: MgmtInterfaces =
//             serde_json::from_str(r#""GigabitEthernet0/0""#).unwrap();
//         assert!(matches!(
//             gigabit_ethernet0_0,
//             MgmtInterfaces::GigabitEthernet0_0
//         ));

//         let mgmteth0rp0cpu0_0: MgmtInterfaces =
//             serde_json::from_str(r#""MgmtEth0/RP0/CPU0/0""#).unwrap();
//         assert!(matches!(
//             mgmteth0rp0cpu0_0,
//             MgmtInterfaces::MgmtEth0Rp0Cpu0_0
//         ));

//         let re0mgmt0: MgmtInterfaces = serde_json::from_str(r#""re0:mgmt-0""#).unwrap();
//         assert!(matches!(re0mgmt0, MgmtInterfaces::Re0Mgmt0));

//         let fxp0: MgmtInterfaces = serde_json::from_str(r#""fxp0""#).unwrap();
//         assert!(matches!(fxp0, MgmtInterfaces::Fxp0));

//         let fxp0_0: MgmtInterfaces = serde_json::from_str(r#""fxp0.0""#).unwrap();
//         assert!(matches!(fxp0_0, MgmtInterfaces::Fxp0_0));

//         let vlan1: MgmtInterfaces = serde_json::from_str(r#""Vlan1""#).unwrap();
//         assert!(matches!(vlan1, MgmtInterfaces::Vlan1));
//     }

//     #[test]
//     fn test_mgmt_interfaces_deserialization_error() {
//         let result: Result<MgmtInterfaces, _> = serde_json::from_str(r#""invalid""#);
//         assert!(result.is_err());
//     }
// }
