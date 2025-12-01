use std::fmt;
use std::str::FromStr;

use serde_derive::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
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
    #[serde(rename = "Vlan1")]
    Vlan1, // Vlan1 - iosvl2
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
            MgmtInterfaces::Vlan1 => write!(f, "Vlan1"),
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionTypes {
    #[default]
    Disabled, // Disable interface
    Management, // Connects to management bridge
    Peer,       // Peered with another device
    Reserved,   // Reserved interfaces used by the virtual platform
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum InterfaceKind {
    Ethernet(EthernetInt),
    ArubaAoscx(ArubaAoscxInt),
    AristaVeos(AristaVeosInt),
    CiscoIosv(CiscoIosvInt),
    CiscoIosvl2(CiscoIosvl2Int),
    CiscoAsav(CiscoAsavInt),
    CiscoCsr1000v(CiscoCsr1000vInt),
    CiscoCat8000v(CiscoCat8000vInt),
    CiscoCat9000v(CiscoCat9000vInt),
    CiscoNexus9000v(CiscoNexus9300vInt),
    JuniperVrouter(JuniperVrouterInt),
    JuniperVswitch(JuniperVswitchInt),
    JuniperVevolved(JuniperVevolvedInt),
    JuniperVsrxv3(JuniperVsrxv3Int),
    CumulusLinux(CumulusLinuxInt),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ArubaAoscxInt {
    #[serde(rename = "1/1/1")]
    E1_1_1,
    #[serde(rename = "1/1/2")]
    E1_1_2,
    #[serde(rename = "1/1/3")]
    E1_1_3,
    #[serde(rename = "1/1/4")]
    E1_1_4,
    #[serde(rename = "1/1/5")]
    E1_1_5,
    #[serde(rename = "1/1/6")]
    E1_1_6,
    #[serde(rename = "1/1/7")]
    E1_1_7,
    #[serde(rename = "1/1/8")]
    E1_1_8,
    #[serde(rename = "1/1/9")]
    E1_1_9,
    #[serde(rename = "1/1/10")]
    E1_1_10,
    #[serde(rename = "1/1/11")]
    E1_1_11,
    #[serde(rename = "1/1/12")]
    E1_1_12,
    #[serde(rename = "1/1/13")]
    E1_1_13,
    #[serde(rename = "1/1/14")]
    E1_1_14,
    #[serde(rename = "1/1/15")]
    E1_1_15,
    #[serde(rename = "1/1/16")]
    E1_1_16,
    #[serde(rename = "1/1/17")]
    E1_1_17,
    #[serde(rename = "1/1/18")]
    E1_1_18,
    #[serde(rename = "1/1/19")]
    E1_1_19,
    #[serde(rename = "1/1/20")]
    E1_1_20,
    #[serde(rename = "1/1/21")]
    E1_1_21,
    #[serde(rename = "1/1/22")]
    E1_1_22,
    #[serde(rename = "1/1/23")]
    E1_1_23,
    #[serde(rename = "1/1/24")]
    E1_1_24,
    #[serde(rename = "1/1/25")]
    E1_1_25,
    #[serde(rename = "1/1/26")]
    E1_1_26,
    #[serde(rename = "1/1/27")]
    E1_1_27,
    #[serde(rename = "1/1/28")]
    E1_1_28,
    #[serde(rename = "1/1/29")]
    E1_1_29,
    #[serde(rename = "1/1/30")]
    E1_1_30,
    #[serde(rename = "1/1/31")]
    E1_1_31,
    #[serde(rename = "1/1/32")]
    E1_1_32,
    #[serde(rename = "1/1/33")]
    E1_1_33,
    #[serde(rename = "1/1/34")]
    E1_1_34,
    #[serde(rename = "1/1/35")]
    E1_1_35,
    #[serde(rename = "1/1/36")]
    E1_1_36,
    #[serde(rename = "1/1/37")]
    E1_1_37,
    #[serde(rename = "1/1/38")]
    E1_1_38,
    #[serde(rename = "1/1/39")]
    E1_1_39,
    #[serde(rename = "1/1/40")]
    E1_1_40,
    #[serde(rename = "1/1/41")]
    E1_1_41,
    #[serde(rename = "1/1/42")]
    E1_1_42,
    #[serde(rename = "1/1/43")]
    E1_1_43,
    #[serde(rename = "1/1/44")]
    E1_1_44,
    #[serde(rename = "1/1/45")]
    E1_1_45,
    #[serde(rename = "1/1/46")]
    E1_1_46,
    #[serde(rename = "1/1/47")]
    E1_1_47,
    #[serde(rename = "1/1/48")]
    E1_1_48,
    #[serde(rename = "1/1/49")]
    E1_1_49,
    #[serde(rename = "1/1/50")]
    E1_1_50,
    #[serde(rename = "1/1/51")]
    E1_1_51,
    #[serde(rename = "1/1/52")]
    E1_1_52,
}

impl InterfaceTrait for ArubaAoscxInt {
    fn to_idx(&self) -> u8 {
        match self {
            ArubaAoscxInt::E1_1_1 => 1,
            ArubaAoscxInt::E1_1_2 => 2,
            ArubaAoscxInt::E1_1_3 => 3,
            ArubaAoscxInt::E1_1_4 => 4,
            ArubaAoscxInt::E1_1_5 => 5,
            ArubaAoscxInt::E1_1_6 => 6,
            ArubaAoscxInt::E1_1_7 => 7,
            ArubaAoscxInt::E1_1_8 => 8,
            ArubaAoscxInt::E1_1_9 => 9,
            ArubaAoscxInt::E1_1_10 => 10,
            ArubaAoscxInt::E1_1_11 => 11,
            ArubaAoscxInt::E1_1_12 => 12,
            ArubaAoscxInt::E1_1_13 => 13,
            ArubaAoscxInt::E1_1_14 => 14,
            ArubaAoscxInt::E1_1_15 => 15,
            ArubaAoscxInt::E1_1_16 => 16,
            ArubaAoscxInt::E1_1_17 => 17,
            ArubaAoscxInt::E1_1_18 => 18,
            ArubaAoscxInt::E1_1_19 => 19,
            ArubaAoscxInt::E1_1_20 => 20,
            ArubaAoscxInt::E1_1_21 => 21,
            ArubaAoscxInt::E1_1_22 => 22,
            ArubaAoscxInt::E1_1_23 => 23,
            ArubaAoscxInt::E1_1_24 => 24,
            ArubaAoscxInt::E1_1_25 => 25,
            ArubaAoscxInt::E1_1_26 => 26,
            ArubaAoscxInt::E1_1_27 => 27,
            ArubaAoscxInt::E1_1_28 => 28,
            ArubaAoscxInt::E1_1_29 => 29,
            ArubaAoscxInt::E1_1_30 => 30,
            ArubaAoscxInt::E1_1_31 => 31,
            ArubaAoscxInt::E1_1_32 => 32,
            ArubaAoscxInt::E1_1_33 => 33,
            ArubaAoscxInt::E1_1_34 => 34,
            ArubaAoscxInt::E1_1_35 => 35,
            ArubaAoscxInt::E1_1_36 => 36,
            ArubaAoscxInt::E1_1_37 => 37,
            ArubaAoscxInt::E1_1_38 => 38,
            ArubaAoscxInt::E1_1_39 => 39,
            ArubaAoscxInt::E1_1_40 => 40,
            ArubaAoscxInt::E1_1_41 => 41,
            ArubaAoscxInt::E1_1_42 => 42,
            ArubaAoscxInt::E1_1_43 => 43,
            ArubaAoscxInt::E1_1_44 => 44,
            ArubaAoscxInt::E1_1_45 => 45,
            ArubaAoscxInt::E1_1_46 => 46,
            ArubaAoscxInt::E1_1_47 => 47,
            ArubaAoscxInt::E1_1_48 => 48,
            ArubaAoscxInt::E1_1_49 => 49,
            ArubaAoscxInt::E1_1_50 => 50,
            ArubaAoscxInt::E1_1_51 => 51,
            ArubaAoscxInt::E1_1_52 => 52,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            1..=52 => format!("1/1/{}", idx),
            _ => String::new(),
        };
        let result = if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "ArubaAoscx",
                idx,
            })
        } else {
            Ok(iface)
        };
        result
    }
}

impl FromStr for ArubaAoscxInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "1/1/1" => Ok(ArubaAoscxInt::E1_1_1),
            "1/1/2" => Ok(ArubaAoscxInt::E1_1_2),
            "1/1/3" => Ok(ArubaAoscxInt::E1_1_3),
            "1/1/4" => Ok(ArubaAoscxInt::E1_1_4),
            "1/1/5" => Ok(ArubaAoscxInt::E1_1_5),
            "1/1/6" => Ok(ArubaAoscxInt::E1_1_6),
            "1/1/7" => Ok(ArubaAoscxInt::E1_1_7),
            "1/1/8" => Ok(ArubaAoscxInt::E1_1_8),
            "1/1/9" => Ok(ArubaAoscxInt::E1_1_9),
            "1/1/10" => Ok(ArubaAoscxInt::E1_1_10),
            "1/1/11" => Ok(ArubaAoscxInt::E1_1_11),
            "1/1/12" => Ok(ArubaAoscxInt::E1_1_12),
            "1/1/13" => Ok(ArubaAoscxInt::E1_1_13),
            "1/1/14" => Ok(ArubaAoscxInt::E1_1_14),
            "1/1/15" => Ok(ArubaAoscxInt::E1_1_15),
            "1/1/16" => Ok(ArubaAoscxInt::E1_1_16),
            "1/1/17" => Ok(ArubaAoscxInt::E1_1_17),
            "1/1/18" => Ok(ArubaAoscxInt::E1_1_18),
            "1/1/19" => Ok(ArubaAoscxInt::E1_1_19),
            "1/1/20" => Ok(ArubaAoscxInt::E1_1_20),
            "1/1/21" => Ok(ArubaAoscxInt::E1_1_21),
            "1/1/22" => Ok(ArubaAoscxInt::E1_1_22),
            "1/1/23" => Ok(ArubaAoscxInt::E1_1_23),
            "1/1/24" => Ok(ArubaAoscxInt::E1_1_24),
            "1/1/25" => Ok(ArubaAoscxInt::E1_1_25),
            "1/1/26" => Ok(ArubaAoscxInt::E1_1_26),
            "1/1/27" => Ok(ArubaAoscxInt::E1_1_27),
            "1/1/28" => Ok(ArubaAoscxInt::E1_1_28),
            "1/1/29" => Ok(ArubaAoscxInt::E1_1_29),
            "1/1/30" => Ok(ArubaAoscxInt::E1_1_30),
            "1/1/31" => Ok(ArubaAoscxInt::E1_1_31),
            "1/1/32" => Ok(ArubaAoscxInt::E1_1_32),
            "1/1/33" => Ok(ArubaAoscxInt::E1_1_33),
            "1/1/34" => Ok(ArubaAoscxInt::E1_1_34),
            "1/1/35" => Ok(ArubaAoscxInt::E1_1_35),
            "1/1/36" => Ok(ArubaAoscxInt::E1_1_36),
            "1/1/37" => Ok(ArubaAoscxInt::E1_1_37),
            "1/1/38" => Ok(ArubaAoscxInt::E1_1_38),
            "1/1/39" => Ok(ArubaAoscxInt::E1_1_39),
            "1/1/40" => Ok(ArubaAoscxInt::E1_1_40),
            "1/1/41" => Ok(ArubaAoscxInt::E1_1_41),
            "1/1/42" => Ok(ArubaAoscxInt::E1_1_42),
            "1/1/43" => Ok(ArubaAoscxInt::E1_1_43),
            "1/1/44" => Ok(ArubaAoscxInt::E1_1_44),
            "1/1/45" => Ok(ArubaAoscxInt::E1_1_45),
            "1/1/46" => Ok(ArubaAoscxInt::E1_1_46),
            "1/1/47" => Ok(ArubaAoscxInt::E1_1_47),
            "1/1/48" => Ok(ArubaAoscxInt::E1_1_48),
            "1/1/49" => Ok(ArubaAoscxInt::E1_1_49),
            "1/1/50" => Ok(ArubaAoscxInt::E1_1_50),
            "1/1/51" => Ok(ArubaAoscxInt::E1_1_51),
            "1/1/52" => Ok(ArubaAoscxInt::E1_1_52),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "ArubaAoscx",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum EthernetInt {
    Eth0,
    Eth1,
    Eth2,
    Eth3,
    Eth4,
    Eth5,
    Eth6,
    Eth7,
    Eth8,
    Eth9,
    Eth10,
    Eth11,
    Eth12,
    Eth13,
    Eth14,
    Eth15,
    Eth16,
    Eth17,
    Eth18,
    Eth19,
    Eth20,
    Eth21,
    Eth22,
    Eth23,
    Eth24,
    Eth25,
    Eth26,
    Eth27,
    Eth28,
    Eth29,
    Eth30,
    Eth31,
    Eth32,
    Eth33,
    Eth34,
    Eth35,
    Eth36,
    Eth37,
    Eth38,
    Eth39,
    Eth40,
    Eth41,
    Eth42,
    Eth43,
    Eth44,
    Eth45,
    Eth46,
    Eth47,
    Eth48,
}
impl InterfaceTrait for EthernetInt {
    fn to_idx(&self) -> u8 {
        match self {
            EthernetInt::Eth0 => 0,
            EthernetInt::Eth1 => 1,
            EthernetInt::Eth2 => 2,
            EthernetInt::Eth3 => 3,
            EthernetInt::Eth4 => 4,
            EthernetInt::Eth5 => 5,
            EthernetInt::Eth6 => 6,
            EthernetInt::Eth7 => 7,
            EthernetInt::Eth8 => 8,
            EthernetInt::Eth9 => 9,
            EthernetInt::Eth10 => 10,
            EthernetInt::Eth11 => 11,
            EthernetInt::Eth12 => 12,
            EthernetInt::Eth13 => 13,
            EthernetInt::Eth14 => 14,
            EthernetInt::Eth15 => 15,
            EthernetInt::Eth16 => 16,
            EthernetInt::Eth17 => 17,
            EthernetInt::Eth18 => 18,
            EthernetInt::Eth19 => 19,
            EthernetInt::Eth20 => 20,
            EthernetInt::Eth21 => 21,
            EthernetInt::Eth22 => 22,
            EthernetInt::Eth23 => 23,
            EthernetInt::Eth24 => 24,
            EthernetInt::Eth25 => 25,
            EthernetInt::Eth26 => 26,
            EthernetInt::Eth27 => 27,
            EthernetInt::Eth28 => 28,
            EthernetInt::Eth29 => 29,
            EthernetInt::Eth30 => 30,
            EthernetInt::Eth31 => 31,
            EthernetInt::Eth32 => 32,
            EthernetInt::Eth33 => 33,
            EthernetInt::Eth34 => 34,
            EthernetInt::Eth35 => 35,
            EthernetInt::Eth36 => 36,
            EthernetInt::Eth37 => 37,
            EthernetInt::Eth38 => 38,
            EthernetInt::Eth39 => 39,
            EthernetInt::Eth40 => 40,
            EthernetInt::Eth41 => 41,
            EthernetInt::Eth42 => 42,
            EthernetInt::Eth43 => 43,
            EthernetInt::Eth44 => 44,
            EthernetInt::Eth45 => 45,
            EthernetInt::Eth46 => 46,
            EthernetInt::Eth47 => 47,
            EthernetInt::Eth48 => 48,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            1..=48 => format!("eth{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "EthernetInt",
                idx,
            })
        } else {
            Ok(iface.to_string())
        }
    }
}
impl FromStr for EthernetInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "eth0" => Ok(EthernetInt::Eth0),
            "eth1" => Ok(EthernetInt::Eth1),
            "eth2" => Ok(EthernetInt::Eth2),
            "eth3" => Ok(EthernetInt::Eth3),
            "eth4" => Ok(EthernetInt::Eth4),
            "eth5" => Ok(EthernetInt::Eth5),
            "eth6" => Ok(EthernetInt::Eth6),
            "eth7" => Ok(EthernetInt::Eth7),
            "eth8" => Ok(EthernetInt::Eth8),
            "eth9" => Ok(EthernetInt::Eth9),
            "eth10" => Ok(EthernetInt::Eth10),
            "eth11" => Ok(EthernetInt::Eth11),
            "eth12" => Ok(EthernetInt::Eth12),
            "eth13" => Ok(EthernetInt::Eth13),
            "eth14" => Ok(EthernetInt::Eth14),
            "eth15" => Ok(EthernetInt::Eth15),
            "eth16" => Ok(EthernetInt::Eth16),
            "eth17" => Ok(EthernetInt::Eth17),
            "eth18" => Ok(EthernetInt::Eth18),
            "eth19" => Ok(EthernetInt::Eth19),
            "eth20" => Ok(EthernetInt::Eth20),
            "eth21" => Ok(EthernetInt::Eth21),
            "eth22" => Ok(EthernetInt::Eth22),
            "eth23" => Ok(EthernetInt::Eth23),
            "eth24" => Ok(EthernetInt::Eth24),
            "eth25" => Ok(EthernetInt::Eth25),
            "eth26" => Ok(EthernetInt::Eth26),
            "eth27" => Ok(EthernetInt::Eth27),
            "eth28" => Ok(EthernetInt::Eth28),
            "eth29" => Ok(EthernetInt::Eth29),
            "eth30" => Ok(EthernetInt::Eth30),
            "eth31" => Ok(EthernetInt::Eth31),
            "eth32" => Ok(EthernetInt::Eth32),
            "eth33" => Ok(EthernetInt::Eth33),
            "eth34" => Ok(EthernetInt::Eth34),
            "eth35" => Ok(EthernetInt::Eth35),
            "eth36" => Ok(EthernetInt::Eth36),
            "eth37" => Ok(EthernetInt::Eth37),
            "eth38" => Ok(EthernetInt::Eth38),
            "eth39" => Ok(EthernetInt::Eth39),
            "eth40" => Ok(EthernetInt::Eth40),
            "eth41" => Ok(EthernetInt::Eth41),
            "eth42" => Ok(EthernetInt::Eth42),
            "eth43" => Ok(EthernetInt::Eth43),
            "eth44" => Ok(EthernetInt::Eth44),
            "eth45" => Ok(EthernetInt::Eth45),
            "eth46" => Ok(EthernetInt::Eth46),
            "eth47" => Ok(EthernetInt::Eth47),
            "eth48" => Ok(EthernetInt::Eth48),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "Generic",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CumulusLinuxInt {
    #[serde(rename = "swp1")]
    Swp1,
    #[serde(rename = "swp2")]
    Swp2,
    #[serde(rename = "swp3")]
    Swp3,
    #[serde(rename = "swp4")]
    Swp4,
    #[serde(rename = "swp5")]
    Swp5,
    #[serde(rename = "swp6")]
    Swp6,
    #[serde(rename = "swp7")]
    Swp7,
    #[serde(rename = "swp8")]
    Swp8,
    #[serde(rename = "swp9")]
    Swp9,
    #[serde(rename = "swp10")]
    Swp10,
    #[serde(rename = "swp11")]
    Swp11,
    #[serde(rename = "swp12")]
    Swp12,
    #[serde(rename = "swp13")]
    Swp13,
    #[serde(rename = "swp14")]
    Swp14,
    #[serde(rename = "swp15")]
    Swp15,
    #[serde(rename = "swp16")]
    Swp16,
    #[serde(rename = "swp17")]
    Swp17,
    #[serde(rename = "swp18")]
    Swp18,
    #[serde(rename = "swp19")]
    Swp19,
    #[serde(rename = "swp20")]
    Swp20,
    #[serde(rename = "swp21")]
    Swp21,
    #[serde(rename = "swp22")]
    Swp22,
    #[serde(rename = "swp23")]
    Swp23,
    #[serde(rename = "swp24")]
    Swp24,
    #[serde(rename = "swp25")]
    Swp25,
    #[serde(rename = "swp26")]
    Swp26,
    #[serde(rename = "swp27")]
    Swp27,
    #[serde(rename = "swp28")]
    Swp28,
    #[serde(rename = "swp29")]
    Swp29,
    #[serde(rename = "swp30")]
    Swp30,
    #[serde(rename = "swp31")]
    Swp31,
    #[serde(rename = "swp32")]
    Swp32,
    #[serde(rename = "swp33")]
    Swp33,
    #[serde(rename = "swp34")]
    Swp34,
    #[serde(rename = "swp35")]
    Swp35,
    #[serde(rename = "swp36")]
    Swp36,
    #[serde(rename = "swp37")]
    Swp37,
    #[serde(rename = "swp38")]
    Swp38,
    #[serde(rename = "swp39")]
    Swp39,
    #[serde(rename = "swp40")]
    Swp40,
    #[serde(rename = "swp41")]
    Swp41,
    #[serde(rename = "swp42")]
    Swp42,
    #[serde(rename = "swp43")]
    Swp43,
    #[serde(rename = "swp44")]
    Swp44,
    #[serde(rename = "swp45")]
    Swp45,
    #[serde(rename = "swp46")]
    Swp46,
    #[serde(rename = "swp47")]
    Swp47,
    #[serde(rename = "swp48")]
    Swp48,
    #[serde(rename = "swp49")]
    Swp49,
    #[serde(rename = "swp50")]
    Swp50,
    #[serde(rename = "swp51")]
    Swp51,
    #[serde(rename = "swp52")]
    Swp52,
}

impl InterfaceTrait for CumulusLinuxInt {
    fn to_idx(&self) -> u8 {
        match self {
            CumulusLinuxInt::Swp1 => 1,
            CumulusLinuxInt::Swp2 => 2,
            CumulusLinuxInt::Swp3 => 3,
            CumulusLinuxInt::Swp4 => 4,
            CumulusLinuxInt::Swp5 => 5,
            CumulusLinuxInt::Swp6 => 6,
            CumulusLinuxInt::Swp7 => 7,
            CumulusLinuxInt::Swp8 => 8,
            CumulusLinuxInt::Swp9 => 9,
            CumulusLinuxInt::Swp10 => 10,
            CumulusLinuxInt::Swp11 => 11,
            CumulusLinuxInt::Swp12 => 12,
            CumulusLinuxInt::Swp13 => 13,
            CumulusLinuxInt::Swp14 => 14,
            CumulusLinuxInt::Swp15 => 15,
            CumulusLinuxInt::Swp16 => 16,
            CumulusLinuxInt::Swp17 => 17,
            CumulusLinuxInt::Swp18 => 18,
            CumulusLinuxInt::Swp19 => 19,
            CumulusLinuxInt::Swp20 => 20,
            CumulusLinuxInt::Swp21 => 21,
            CumulusLinuxInt::Swp22 => 22,
            CumulusLinuxInt::Swp23 => 23,
            CumulusLinuxInt::Swp24 => 24,
            CumulusLinuxInt::Swp25 => 25,
            CumulusLinuxInt::Swp26 => 26,
            CumulusLinuxInt::Swp27 => 27,
            CumulusLinuxInt::Swp28 => 28,
            CumulusLinuxInt::Swp29 => 29,
            CumulusLinuxInt::Swp30 => 30,
            CumulusLinuxInt::Swp31 => 31,
            CumulusLinuxInt::Swp32 => 32,
            CumulusLinuxInt::Swp33 => 33,
            CumulusLinuxInt::Swp34 => 34,
            CumulusLinuxInt::Swp35 => 35,
            CumulusLinuxInt::Swp36 => 36,
            CumulusLinuxInt::Swp37 => 37,
            CumulusLinuxInt::Swp38 => 38,
            CumulusLinuxInt::Swp39 => 39,
            CumulusLinuxInt::Swp40 => 40,
            CumulusLinuxInt::Swp41 => 41,
            CumulusLinuxInt::Swp42 => 42,
            CumulusLinuxInt::Swp43 => 43,
            CumulusLinuxInt::Swp44 => 44,
            CumulusLinuxInt::Swp45 => 45,
            CumulusLinuxInt::Swp46 => 46,
            CumulusLinuxInt::Swp47 => 47,
            CumulusLinuxInt::Swp48 => 48,
            CumulusLinuxInt::Swp49 => 49,
            CumulusLinuxInt::Swp50 => 50,
            CumulusLinuxInt::Swp51 => 51,
            CumulusLinuxInt::Swp52 => 52,
        }
    }

    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            1..=52 => format!("swp{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "CumulusLinuxInt",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}

impl FromStr for CumulusLinuxInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "swp1" => Ok(CumulusLinuxInt::Swp1),
            "swp2" => Ok(CumulusLinuxInt::Swp2),
            "swp3" => Ok(CumulusLinuxInt::Swp3),
            "swp4" => Ok(CumulusLinuxInt::Swp4),
            "swp5" => Ok(CumulusLinuxInt::Swp5),
            "swp6" => Ok(CumulusLinuxInt::Swp6),
            "swp7" => Ok(CumulusLinuxInt::Swp7),
            "swp8" => Ok(CumulusLinuxInt::Swp8),
            "swp9" => Ok(CumulusLinuxInt::Swp9),
            "swp10" => Ok(CumulusLinuxInt::Swp10),
            "swp11" => Ok(CumulusLinuxInt::Swp11),
            "swp12" => Ok(CumulusLinuxInt::Swp12),
            "swp13" => Ok(CumulusLinuxInt::Swp13),
            "swp14" => Ok(CumulusLinuxInt::Swp14),
            "swp15" => Ok(CumulusLinuxInt::Swp15),
            "swp16" => Ok(CumulusLinuxInt::Swp16),
            "swp17" => Ok(CumulusLinuxInt::Swp17),
            "swp18" => Ok(CumulusLinuxInt::Swp18),
            "swp19" => Ok(CumulusLinuxInt::Swp19),
            "swp20" => Ok(CumulusLinuxInt::Swp20),
            "swp21" => Ok(CumulusLinuxInt::Swp21),
            "swp22" => Ok(CumulusLinuxInt::Swp22),
            "swp23" => Ok(CumulusLinuxInt::Swp23),
            "swp24" => Ok(CumulusLinuxInt::Swp24),
            "swp25" => Ok(CumulusLinuxInt::Swp25),
            "swp26" => Ok(CumulusLinuxInt::Swp26),
            "swp27" => Ok(CumulusLinuxInt::Swp27),
            "swp28" => Ok(CumulusLinuxInt::Swp28),
            "swp29" => Ok(CumulusLinuxInt::Swp29),
            "swp30" => Ok(CumulusLinuxInt::Swp30),
            "swp31" => Ok(CumulusLinuxInt::Swp31),
            "swp32" => Ok(CumulusLinuxInt::Swp32),
            "swp33" => Ok(CumulusLinuxInt::Swp33),
            "swp34" => Ok(CumulusLinuxInt::Swp34),
            "swp35" => Ok(CumulusLinuxInt::Swp35),
            "swp36" => Ok(CumulusLinuxInt::Swp36),
            "swp37" => Ok(CumulusLinuxInt::Swp37),
            "swp38" => Ok(CumulusLinuxInt::Swp38),
            "swp39" => Ok(CumulusLinuxInt::Swp39),
            "swp40" => Ok(CumulusLinuxInt::Swp40),
            "swp41" => Ok(CumulusLinuxInt::Swp41),
            "swp42" => Ok(CumulusLinuxInt::Swp42),
            "swp43" => Ok(CumulusLinuxInt::Swp43),
            "swp44" => Ok(CumulusLinuxInt::Swp44),
            "swp45" => Ok(CumulusLinuxInt::Swp45),
            "swp46" => Ok(CumulusLinuxInt::Swp46),
            "swp47" => Ok(CumulusLinuxInt::Swp47),
            "swp48" => Ok(CumulusLinuxInt::Swp48),
            "swp49" => Ok(CumulusLinuxInt::Swp49),
            "swp50" => Ok(CumulusLinuxInt::Swp50),
            "swp51" => Ok(CumulusLinuxInt::Swp51),
            "swp52" => Ok(CumulusLinuxInt::Swp52),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "CumulusLinuxInt",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum AristaVeosInt {
    Eth1,
    Eth2,
    Eth3,
    Eth4,
    Eth5,
    Eth6,
    Eth7,
    Eth8,
    Eth9,
    Eth10,
    Eth11,
    Eth12,
    Eth13,
    Eth14,
    Eth15,
    Eth16,
    Eth17,
    Eth18,
    Eth19,
    Eth20,
    Eth21,
    Eth22,
    Eth23,
    Eth24,
    Eth25,
    Eth26,
    Eth27,
    Eth28,
    Eth29,
    Eth30,
    Eth31,
    Eth32,
    Eth33,
    Eth34,
    Eth35,
    Eth36,
    Eth37,
    Eth38,
    Eth39,
    Eth40,
    Eth41,
    Eth42,
    Eth43,
    Eth44,
    Eth45,
    Eth46,
    Eth47,
    Eth48,
}
impl InterfaceTrait for AristaVeosInt {
    fn to_idx(&self) -> u8 {
        match self {
            AristaVeosInt::Eth1 => 1,
            AristaVeosInt::Eth2 => 2,
            AristaVeosInt::Eth3 => 3,
            AristaVeosInt::Eth4 => 4,
            AristaVeosInt::Eth5 => 5,
            AristaVeosInt::Eth6 => 6,
            AristaVeosInt::Eth7 => 7,
            AristaVeosInt::Eth8 => 8,
            AristaVeosInt::Eth9 => 9,
            AristaVeosInt::Eth10 => 10,
            AristaVeosInt::Eth11 => 11,
            AristaVeosInt::Eth12 => 12,
            AristaVeosInt::Eth13 => 13,
            AristaVeosInt::Eth14 => 14,
            AristaVeosInt::Eth15 => 15,
            AristaVeosInt::Eth16 => 16,
            AristaVeosInt::Eth17 => 17,
            AristaVeosInt::Eth18 => 18,
            AristaVeosInt::Eth19 => 19,
            AristaVeosInt::Eth20 => 20,
            AristaVeosInt::Eth21 => 21,
            AristaVeosInt::Eth22 => 22,
            AristaVeosInt::Eth23 => 23,
            AristaVeosInt::Eth24 => 24,
            AristaVeosInt::Eth25 => 25,
            AristaVeosInt::Eth26 => 26,
            AristaVeosInt::Eth27 => 27,
            AristaVeosInt::Eth28 => 28,
            AristaVeosInt::Eth29 => 29,
            AristaVeosInt::Eth30 => 30,
            AristaVeosInt::Eth31 => 31,
            AristaVeosInt::Eth32 => 32,
            AristaVeosInt::Eth33 => 33,
            AristaVeosInt::Eth34 => 34,
            AristaVeosInt::Eth35 => 35,
            AristaVeosInt::Eth36 => 36,
            AristaVeosInt::Eth37 => 37,
            AristaVeosInt::Eth38 => 38,
            AristaVeosInt::Eth39 => 39,
            AristaVeosInt::Eth40 => 40,
            AristaVeosInt::Eth41 => 41,
            AristaVeosInt::Eth42 => 42,
            AristaVeosInt::Eth43 => 43,
            AristaVeosInt::Eth44 => 44,
            AristaVeosInt::Eth45 => 45,
            AristaVeosInt::Eth46 => 46,
            AristaVeosInt::Eth47 => 47,
            AristaVeosInt::Eth48 => 48,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            1..=48 => format!("eth{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "AristaVeos",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for AristaVeosInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "eth1" => Ok(AristaVeosInt::Eth1),
            "eth2" => Ok(AristaVeosInt::Eth2),
            "eth3" => Ok(AristaVeosInt::Eth3),
            "eth4" => Ok(AristaVeosInt::Eth4),
            "eth5" => Ok(AristaVeosInt::Eth5),
            "eth6" => Ok(AristaVeosInt::Eth6),
            "eth7" => Ok(AristaVeosInt::Eth7),
            "eth8" => Ok(AristaVeosInt::Eth8),
            "eth9" => Ok(AristaVeosInt::Eth9),
            "eth10" => Ok(AristaVeosInt::Eth10),
            "eth11" => Ok(AristaVeosInt::Eth11),
            "eth12" => Ok(AristaVeosInt::Eth12),
            "eth13" => Ok(AristaVeosInt::Eth13),
            "eth14" => Ok(AristaVeosInt::Eth14),
            "eth15" => Ok(AristaVeosInt::Eth15),
            "eth16" => Ok(AristaVeosInt::Eth16),
            "eth17" => Ok(AristaVeosInt::Eth17),
            "eth18" => Ok(AristaVeosInt::Eth18),
            "eth19" => Ok(AristaVeosInt::Eth19),
            "eth20" => Ok(AristaVeosInt::Eth20),
            "eth21" => Ok(AristaVeosInt::Eth21),
            "eth22" => Ok(AristaVeosInt::Eth22),
            "eth23" => Ok(AristaVeosInt::Eth23),
            "eth24" => Ok(AristaVeosInt::Eth24),
            "eth25" => Ok(AristaVeosInt::Eth25),
            "eth26" => Ok(AristaVeosInt::Eth26),
            "eth27" => Ok(AristaVeosInt::Eth27),
            "eth28" => Ok(AristaVeosInt::Eth28),
            "eth29" => Ok(AristaVeosInt::Eth29),
            "eth30" => Ok(AristaVeosInt::Eth30),
            "eth31" => Ok(AristaVeosInt::Eth31),
            "eth32" => Ok(AristaVeosInt::Eth32),
            "eth33" => Ok(AristaVeosInt::Eth33),
            "eth34" => Ok(AristaVeosInt::Eth34),
            "eth35" => Ok(AristaVeosInt::Eth35),
            "eth36" => Ok(AristaVeosInt::Eth36),
            "eth37" => Ok(AristaVeosInt::Eth37),
            "eth38" => Ok(AristaVeosInt::Eth38),
            "eth39" => Ok(AristaVeosInt::Eth39),
            "eth40" => Ok(AristaVeosInt::Eth40),
            "eth41" => Ok(AristaVeosInt::Eth41),
            "eth42" => Ok(AristaVeosInt::Eth42),
            "eth43" => Ok(AristaVeosInt::Eth43),
            "eth44" => Ok(AristaVeosInt::Eth44),
            "eth45" => Ok(AristaVeosInt::Eth45),
            "eth46" => Ok(AristaVeosInt::Eth46),
            "eth47" => Ok(AristaVeosInt::Eth47),
            "eth48" => Ok(AristaVeosInt::Eth48),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "AristaVeos",
                iface: text.to_string(),
            }),
        }
    }
}

pub enum AristaCeosInt {
    Eth1,
    Eth2,
    Eth3,
    Eth4,
    Eth5,
    Eth6,
    Eth7,
    Eth8,
    Eth9,
    Eth10,
    Eth11,
    Eth12,
    Eth13,
    Eth14,
    Eth15,
    Eth16,
    Eth17,
    Eth18,
    Eth19,
    Eth20,
    Eth21,
    Eth22,
    Eth23,
    Eth24,
    Eth25,
    Eth26,
    Eth27,
    Eth28,
    Eth29,
    Eth30,
    Eth31,
    Eth32,
    Eth33,
    Eth34,
    Eth35,
    Eth36,
    Eth37,
    Eth38,
    Eth39,
    Eth40,
    Eth41,
    Eth42,
    Eth43,
    Eth44,
    Eth45,
    Eth46,
    Eth47,
    Eth48,
}
impl InterfaceTrait for AristaCeosInt {
    fn to_idx(&self) -> u8 {
        match self {
            AristaCeosInt::Eth1 => 1,
            AristaCeosInt::Eth2 => 2,
            AristaCeosInt::Eth3 => 3,
            AristaCeosInt::Eth4 => 4,
            AristaCeosInt::Eth5 => 5,
            AristaCeosInt::Eth6 => 6,
            AristaCeosInt::Eth7 => 7,
            AristaCeosInt::Eth8 => 8,
            AristaCeosInt::Eth9 => 9,
            AristaCeosInt::Eth10 => 10,
            AristaCeosInt::Eth11 => 11,
            AristaCeosInt::Eth12 => 12,
            AristaCeosInt::Eth13 => 13,
            AristaCeosInt::Eth14 => 14,
            AristaCeosInt::Eth15 => 15,
            AristaCeosInt::Eth16 => 16,
            AristaCeosInt::Eth17 => 17,
            AristaCeosInt::Eth18 => 18,
            AristaCeosInt::Eth19 => 19,
            AristaCeosInt::Eth20 => 20,
            AristaCeosInt::Eth21 => 21,
            AristaCeosInt::Eth22 => 22,
            AristaCeosInt::Eth23 => 23,
            AristaCeosInt::Eth24 => 24,
            AristaCeosInt::Eth25 => 25,
            AristaCeosInt::Eth26 => 26,
            AristaCeosInt::Eth27 => 27,
            AristaCeosInt::Eth28 => 28,
            AristaCeosInt::Eth29 => 29,
            AristaCeosInt::Eth30 => 30,
            AristaCeosInt::Eth31 => 31,
            AristaCeosInt::Eth32 => 32,
            AristaCeosInt::Eth33 => 33,
            AristaCeosInt::Eth34 => 34,
            AristaCeosInt::Eth35 => 35,
            AristaCeosInt::Eth36 => 36,
            AristaCeosInt::Eth37 => 37,
            AristaCeosInt::Eth38 => 38,
            AristaCeosInt::Eth39 => 39,
            AristaCeosInt::Eth40 => 40,
            AristaCeosInt::Eth41 => 41,
            AristaCeosInt::Eth42 => 42,
            AristaCeosInt::Eth43 => 43,
            AristaCeosInt::Eth44 => 44,
            AristaCeosInt::Eth45 => 45,
            AristaCeosInt::Eth46 => 46,
            AristaCeosInt::Eth47 => 47,
            AristaCeosInt::Eth48 => 48,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            1..=48 => format!("eth{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "AristaCeos",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for AristaCeosInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "eth1" => Ok(AristaCeosInt::Eth1),
            "eth2" => Ok(AristaCeosInt::Eth2),
            "eth3" => Ok(AristaCeosInt::Eth3),
            "eth4" => Ok(AristaCeosInt::Eth4),
            "eth5" => Ok(AristaCeosInt::Eth5),
            "eth6" => Ok(AristaCeosInt::Eth6),
            "eth7" => Ok(AristaCeosInt::Eth7),
            "eth8" => Ok(AristaCeosInt::Eth8),
            "eth9" => Ok(AristaCeosInt::Eth9),
            "eth10" => Ok(AristaCeosInt::Eth10),
            "eth11" => Ok(AristaCeosInt::Eth11),
            "eth12" => Ok(AristaCeosInt::Eth12),
            "eth13" => Ok(AristaCeosInt::Eth13),
            "eth14" => Ok(AristaCeosInt::Eth14),
            "eth15" => Ok(AristaCeosInt::Eth15),
            "eth16" => Ok(AristaCeosInt::Eth16),
            "eth17" => Ok(AristaCeosInt::Eth17),
            "eth18" => Ok(AristaCeosInt::Eth18),
            "eth19" => Ok(AristaCeosInt::Eth19),
            "eth20" => Ok(AristaCeosInt::Eth20),
            "eth21" => Ok(AristaCeosInt::Eth21),
            "eth22" => Ok(AristaCeosInt::Eth22),
            "eth23" => Ok(AristaCeosInt::Eth23),
            "eth24" => Ok(AristaCeosInt::Eth24),
            "eth25" => Ok(AristaCeosInt::Eth25),
            "eth26" => Ok(AristaCeosInt::Eth26),
            "eth27" => Ok(AristaCeosInt::Eth27),
            "eth28" => Ok(AristaCeosInt::Eth28),
            "eth29" => Ok(AristaCeosInt::Eth29),
            "eth30" => Ok(AristaCeosInt::Eth30),
            "eth31" => Ok(AristaCeosInt::Eth31),
            "eth32" => Ok(AristaCeosInt::Eth32),
            "eth33" => Ok(AristaCeosInt::Eth33),
            "eth34" => Ok(AristaCeosInt::Eth34),
            "eth35" => Ok(AristaCeosInt::Eth35),
            "eth36" => Ok(AristaCeosInt::Eth36),
            "eth37" => Ok(AristaCeosInt::Eth37),
            "eth38" => Ok(AristaCeosInt::Eth38),
            "eth39" => Ok(AristaCeosInt::Eth39),
            "eth40" => Ok(AristaCeosInt::Eth40),
            "eth41" => Ok(AristaCeosInt::Eth41),
            "eth42" => Ok(AristaCeosInt::Eth42),
            "eth43" => Ok(AristaCeosInt::Eth43),
            "eth44" => Ok(AristaCeosInt::Eth44),
            "eth45" => Ok(AristaCeosInt::Eth45),
            "eth46" => Ok(AristaCeosInt::Eth46),
            "eth47" => Ok(AristaCeosInt::Eth47),
            "eth48" => Ok(AristaCeosInt::Eth48),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "AristaCeos",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CiscoIosvInt {
    #[serde(rename = "gig0/0")]
    Gig0_0,
    #[serde(rename = "gig0/1")]
    Gig0_1,
    #[serde(rename = "gig0/2")]
    Gig0_2,
    #[serde(rename = "gig0/3")]
    Gig0_3,
    #[serde(rename = "gig0/4")]
    Gig0_4,
    #[serde(rename = "gig0/5")]
    Gig0_5,
    #[serde(rename = "gig0/6")]
    Gig0_6,
    #[serde(rename = "gig0/7")]
    Gig0_7,
    #[serde(rename = "gig0/8")]
    Gig0_8,
    #[serde(rename = "gig0/9")]
    Gig0_9,
    #[serde(rename = "gig0/10")]
    Gig0_10,
    #[serde(rename = "gig0/11")]
    Gig0_11,
    #[serde(rename = "gig0/12")]
    Gig0_12,
    #[serde(rename = "gig0/13")]
    Gig0_13,
    #[serde(rename = "gig0/14")]
    Gig0_14,
    #[serde(rename = "gig0/15")]
    Gig0_15,
}
impl InterfaceTrait for CiscoIosvInt {
    fn to_idx(&self) -> u8 {
        match self {
            CiscoIosvInt::Gig0_0 => 0,
            CiscoIosvInt::Gig0_1 => 1,
            CiscoIosvInt::Gig0_2 => 2,
            CiscoIosvInt::Gig0_3 => 3,
            CiscoIosvInt::Gig0_4 => 4,
            CiscoIosvInt::Gig0_5 => 5,
            CiscoIosvInt::Gig0_6 => 6,
            CiscoIosvInt::Gig0_7 => 7,
            CiscoIosvInt::Gig0_8 => 8,
            CiscoIosvInt::Gig0_9 => 9,
            CiscoIosvInt::Gig0_10 => 10,
            CiscoIosvInt::Gig0_11 => 11,
            CiscoIosvInt::Gig0_12 => 12,
            CiscoIosvInt::Gig0_13 => 13,
            CiscoIosvInt::Gig0_14 => 14,
            CiscoIosvInt::Gig0_15 => 15,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            0..=15 => format!("gig0/{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "CiscoIosv",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for CiscoIosvInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "gig0/0" => Ok(CiscoIosvInt::Gig0_0),
            "gig0/1" => Ok(CiscoIosvInt::Gig0_1),
            "gig0/2" => Ok(CiscoIosvInt::Gig0_2),
            "gig0/3" => Ok(CiscoIosvInt::Gig0_3),
            "gig0/4" => Ok(CiscoIosvInt::Gig0_4),
            "gig0/5" => Ok(CiscoIosvInt::Gig0_5),
            "gig0/6" => Ok(CiscoIosvInt::Gig0_6),
            "gig0/7" => Ok(CiscoIosvInt::Gig0_7),
            "gig0/8" => Ok(CiscoIosvInt::Gig0_8),
            "gig0/9" => Ok(CiscoIosvInt::Gig0_9),
            "gig0/10" => Ok(CiscoIosvInt::Gig0_10),
            "gig0/11" => Ok(CiscoIosvInt::Gig0_11),
            "gig0/12" => Ok(CiscoIosvInt::Gig0_12),
            "gig0/13" => Ok(CiscoIosvInt::Gig0_13),
            "gig0/14" => Ok(CiscoIosvInt::Gig0_14),
            "gig0/15" => Ok(CiscoIosvInt::Gig0_15),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "CiscoIosv",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CiscoIosvl2Int {
    #[serde(rename = "gig0/0")]
    Gig0_0,
    #[serde(rename = "gig0/1")]
    Gig0_1,
    #[serde(rename = "gig0/2")]
    Gig0_2,
    #[serde(rename = "gig0/3")]
    Gig0_3,
    #[serde(rename = "gig1/0")]
    Gig1_0,
    #[serde(rename = "gig1/1")]
    Gig1_1,
    #[serde(rename = "gig1/2")]
    Gig1_2,
    #[serde(rename = "gig1/3")]
    Gig1_3,
    #[serde(rename = "gig2/0")]
    Gig2_0,
    #[serde(rename = "gig2/1")]
    Gig2_1,
    #[serde(rename = "gig2/2")]
    Gig2_2,
    #[serde(rename = "gig2/3")]
    Gig2_3,
    #[serde(rename = "gig3/0")]
    Gig3_0,
    #[serde(rename = "gig3/1")]
    Gig3_1,
    #[serde(rename = "gig3/2")]
    Gig3_2,
    #[serde(rename = "gig3/3")]
    Gig3_3,
}
impl InterfaceTrait for CiscoIosvl2Int {
    fn to_idx(&self) -> u8 {
        match self {
            CiscoIosvl2Int::Gig0_0 => 0,
            CiscoIosvl2Int::Gig0_1 => 1,
            CiscoIosvl2Int::Gig0_2 => 2,
            CiscoIosvl2Int::Gig0_3 => 3,
            CiscoIosvl2Int::Gig1_0 => 4,
            CiscoIosvl2Int::Gig1_1 => 5,
            CiscoIosvl2Int::Gig1_2 => 6,
            CiscoIosvl2Int::Gig1_3 => 7,
            CiscoIosvl2Int::Gig2_0 => 8,
            CiscoIosvl2Int::Gig2_1 => 9,
            CiscoIosvl2Int::Gig2_2 => 10,
            CiscoIosvl2Int::Gig2_3 => 11,
            CiscoIosvl2Int::Gig3_0 => 12,
            CiscoIosvl2Int::Gig3_1 => 13,
            CiscoIosvl2Int::Gig3_2 => 14,
            CiscoIosvl2Int::Gig3_3 => 15,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            0 => "gig0/0",
            1 => "gig0/1",
            2 => "gig0/2",
            3 => "gig0/3",
            4 => "gig1/0",
            5 => "gig1/1",
            6 => "gig1/2",
            7 => "gig1/3",
            8 => "gig2/0",
            9 => "gig2/1",
            10 => "gig2/2",
            11 => "gig2/3",
            12 => "gig3/0",
            13 => "gig3/1",
            14 => "gig3/2",
            15 => "gig3/3",
            _ => "",
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "CiscoIosvl2",
                idx,
            })
        } else {
            Ok(iface.to_string())
        }
    }
}
impl FromStr for CiscoIosvl2Int {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "gig0/0" => Ok(CiscoIosvl2Int::Gig0_0),
            "gig0/1" => Ok(CiscoIosvl2Int::Gig0_1),
            "gig0/2" => Ok(CiscoIosvl2Int::Gig0_2),
            "gig0/3" => Ok(CiscoIosvl2Int::Gig0_3),
            "gig1/0" => Ok(CiscoIosvl2Int::Gig1_0),
            "gig1/1" => Ok(CiscoIosvl2Int::Gig1_1),
            "gig1/2" => Ok(CiscoIosvl2Int::Gig1_2),
            "gig1/3" => Ok(CiscoIosvl2Int::Gig1_3),
            "gig2/0" => Ok(CiscoIosvl2Int::Gig2_0),
            "gig2/1" => Ok(CiscoIosvl2Int::Gig2_1),
            "gig2/2" => Ok(CiscoIosvl2Int::Gig2_2),
            "gig2/3" => Ok(CiscoIosvl2Int::Gig2_3),
            "gig3/0" => Ok(CiscoIosvl2Int::Gig3_0),
            "gig3/1" => Ok(CiscoIosvl2Int::Gig3_1),
            "gig3/2" => Ok(CiscoIosvl2Int::Gig3_2),
            "gig3/3" => Ok(CiscoIosvl2Int::Gig3_3),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "CiscoIos",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CiscoIosxrv9000Int {
    #[serde(rename = "gig0/0/0/0")]
    Gig0_0_0_0,
    #[serde(rename = "gig0/0/0/1")]
    Gig0_0_0_1,
    #[serde(rename = "gig0/0/0/2")]
    Gig0_0_0_2,
    #[serde(rename = "gig0/0/0/3")]
    Gig0_0_0_3,
    #[serde(rename = "gig0/0/0/4")]
    Gig0_0_0_4,
    #[serde(rename = "gig0/0/0/5")]
    Gig0_0_0_5,
    #[serde(rename = "gig0/0/0/6")]
    Gig0_0_0_6,
    #[serde(rename = "gig0/0/0/7")]
    Gig0_0_0_7,
    #[serde(rename = "gig0/0/0/8")]
    Gig0_0_0_8,
    #[serde(rename = "gig0/0/0/9")]
    Gig0_0_0_9,
    #[serde(rename = "gig0/0/0/10")]
    Gig0_0_0_10,
    #[serde(rename = "gig0/0/0/11")]
    Gig0_0_0_11,
    #[serde(rename = "gig0/0/0/12")]
    Gig0_0_0_12,
    #[serde(rename = "gig0/0/0/13")]
    Gig0_0_0_13,
    #[serde(rename = "gig0/0/0/14")]
    Gig0_0_0_14,
    #[serde(rename = "gig0/0/0/15")]
    Gig0_0_0_15,
    #[serde(rename = "gig0/0/0/16")]
    Gig0_0_0_16,
    #[serde(rename = "gig0/0/0/17")]
    Gig0_0_0_17,
    #[serde(rename = "gig0/0/0/18")]
    Gig0_0_0_18,
    #[serde(rename = "gig0/0/0/19")]
    Gig0_0_0_19,
    #[serde(rename = "gig0/0/0/20")]
    Gig0_0_0_20,
    #[serde(rename = "gig0/0/0/21")]
    Gig0_0_0_21,
    #[serde(rename = "gig0/0/0/22")]
    Gig0_0_0_22,
    #[serde(rename = "gig0/0/0/23")]
    Gig0_0_0_23,
    #[serde(rename = "gig0/0/0/24")]
    Gig0_0_0_24,
    #[serde(rename = "gig0/0/0/25")]
    Gig0_0_0_25,
    #[serde(rename = "gig0/0/0/26")]
    Gig0_0_0_26,
    #[serde(rename = "gig0/0/0/27")]
    Gig0_0_0_27,
    #[serde(rename = "gig0/0/0/28")]
    Gig0_0_0_28,
    #[serde(rename = "gig0/0/0/29")]
    Gig0_0_0_29,
    #[serde(rename = "gig0/0/0/30")]
    Gig0_0_0_30,
}

impl InterfaceTrait for CiscoIosxrv9000Int {
    fn to_idx(&self) -> u8 {
        match self {
            CiscoIosxrv9000Int::Gig0_0_0_0 => 0,
            CiscoIosxrv9000Int::Gig0_0_0_1 => 1,
            CiscoIosxrv9000Int::Gig0_0_0_2 => 2,
            CiscoIosxrv9000Int::Gig0_0_0_3 => 3,
            CiscoIosxrv9000Int::Gig0_0_0_4 => 4,
            CiscoIosxrv9000Int::Gig0_0_0_5 => 5,
            CiscoIosxrv9000Int::Gig0_0_0_6 => 6,
            CiscoIosxrv9000Int::Gig0_0_0_7 => 7,
            CiscoIosxrv9000Int::Gig0_0_0_8 => 8,
            CiscoIosxrv9000Int::Gig0_0_0_9 => 9,
            CiscoIosxrv9000Int::Gig0_0_0_10 => 10,
            CiscoIosxrv9000Int::Gig0_0_0_11 => 11,
            CiscoIosxrv9000Int::Gig0_0_0_12 => 12,
            CiscoIosxrv9000Int::Gig0_0_0_13 => 13,
            CiscoIosxrv9000Int::Gig0_0_0_14 => 14,
            CiscoIosxrv9000Int::Gig0_0_0_15 => 15,
            CiscoIosxrv9000Int::Gig0_0_0_16 => 16,
            CiscoIosxrv9000Int::Gig0_0_0_17 => 17,
            CiscoIosxrv9000Int::Gig0_0_0_18 => 18,
            CiscoIosxrv9000Int::Gig0_0_0_19 => 19,
            CiscoIosxrv9000Int::Gig0_0_0_20 => 20,
            CiscoIosxrv9000Int::Gig0_0_0_21 => 21,
            CiscoIosxrv9000Int::Gig0_0_0_22 => 22,
            CiscoIosxrv9000Int::Gig0_0_0_23 => 23,
            CiscoIosxrv9000Int::Gig0_0_0_24 => 24,
            CiscoIosxrv9000Int::Gig0_0_0_25 => 25,
            CiscoIosxrv9000Int::Gig0_0_0_26 => 26,
            CiscoIosxrv9000Int::Gig0_0_0_27 => 27,
            CiscoIosxrv9000Int::Gig0_0_0_28 => 28,
            CiscoIosxrv9000Int::Gig0_0_0_29 => 29,
            CiscoIosxrv9000Int::Gig0_0_0_30 => 30,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            0..=30 => format!("gig0/0/0/{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "CiscoIosxrv9000Int",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}

impl FromStr for CiscoIosxrv9000Int {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "gig0/0/0/0" => Ok(CiscoIosxrv9000Int::Gig0_0_0_0),
            "gig0/0/0/1" => Ok(CiscoIosxrv9000Int::Gig0_0_0_1),
            "gig0/0/0/2" => Ok(CiscoIosxrv9000Int::Gig0_0_0_2),
            "gig0/0/0/3" => Ok(CiscoIosxrv9000Int::Gig0_0_0_3),
            "gig0/0/0/4" => Ok(CiscoIosxrv9000Int::Gig0_0_0_4),
            "gig0/0/0/5" => Ok(CiscoIosxrv9000Int::Gig0_0_0_5),
            "gig0/0/0/6" => Ok(CiscoIosxrv9000Int::Gig0_0_0_6),
            "gig0/0/0/7" => Ok(CiscoIosxrv9000Int::Gig0_0_0_7),
            "gig0/0/0/8" => Ok(CiscoIosxrv9000Int::Gig0_0_0_8),
            "gig0/0/0/9" => Ok(CiscoIosxrv9000Int::Gig0_0_0_9),
            "gig0/0/0/10" => Ok(CiscoIosxrv9000Int::Gig0_0_0_10),
            "gig0/0/0/11" => Ok(CiscoIosxrv9000Int::Gig0_0_0_11),
            "gig0/0/0/12" => Ok(CiscoIosxrv9000Int::Gig0_0_0_12),
            "gig0/0/0/13" => Ok(CiscoIosxrv9000Int::Gig0_0_0_13),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "CiscoIosxrv9000Int",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CiscoAsavInt {
    #[serde(rename = "gig0/0")]
    Gig0_0,
    #[serde(rename = "gig0/1")]
    Gig0_1,
    #[serde(rename = "gig0/2")]
    Gig0_2,
    #[serde(rename = "gig0/3")]
    Gig0_3,
    #[serde(rename = "gig0/4")]
    Gig0_4,
    #[serde(rename = "gig0/5")]
    Gig0_5,
    #[serde(rename = "gig0/6")]
    Gig0_6,
    #[serde(rename = "gig0/7")]
    Gig0_7,
}
impl InterfaceTrait for CiscoAsavInt {
    fn to_idx(&self) -> u8 {
        match self {
            CiscoAsavInt::Gig0_0 => 0,
            CiscoAsavInt::Gig0_1 => 1,
            CiscoAsavInt::Gig0_2 => 2,
            CiscoAsavInt::Gig0_3 => 3,
            CiscoAsavInt::Gig0_4 => 4,
            CiscoAsavInt::Gig0_5 => 5,
            CiscoAsavInt::Gig0_6 => 6,
            CiscoAsavInt::Gig0_7 => 7,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            0..=7 => format!("gig0/{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "CiscoAsav",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for CiscoAsavInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "gig0/0" => Ok(CiscoAsavInt::Gig0_0),
            "gig0/1" => Ok(CiscoAsavInt::Gig0_1),
            "gig0/2" => Ok(CiscoAsavInt::Gig0_2),
            "gig0/3" => Ok(CiscoAsavInt::Gig0_3),
            "gig0/4" => Ok(CiscoAsavInt::Gig0_4),
            "gig0/5" => Ok(CiscoAsavInt::Gig0_5),
            "gig0/6" => Ok(CiscoAsavInt::Gig0_6),
            "gig0/7" => Ok(CiscoAsavInt::Gig0_7),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "CiscoAsav",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum CiscoCsr1000vInt {
    Gig1,
    Gig2,
    Gig3,
    Gig4,
    Gig5,
    Gig6,
    Gig7,
    Gig8,
    Gig9,
    Gig10,
    Gig11,
    Gig12,
    Gig13,
    Gig14,
    Gig15,
    Gig16,
}
impl InterfaceTrait for CiscoCsr1000vInt {
    fn to_idx(&self) -> u8 {
        match self {
            CiscoCsr1000vInt::Gig1 => 1,
            CiscoCsr1000vInt::Gig2 => 2,
            CiscoCsr1000vInt::Gig3 => 3,
            CiscoCsr1000vInt::Gig4 => 4,
            CiscoCsr1000vInt::Gig5 => 5,
            CiscoCsr1000vInt::Gig6 => 6,
            CiscoCsr1000vInt::Gig7 => 7,
            CiscoCsr1000vInt::Gig8 => 8,
            CiscoCsr1000vInt::Gig9 => 9,
            CiscoCsr1000vInt::Gig10 => 10,
            CiscoCsr1000vInt::Gig11 => 11,
            CiscoCsr1000vInt::Gig12 => 12,
            CiscoCsr1000vInt::Gig13 => 13,
            CiscoCsr1000vInt::Gig14 => 14,
            CiscoCsr1000vInt::Gig15 => 15,
            CiscoCsr1000vInt::Gig16 => 16,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            1..=16 => format!("gig{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "CiscoCsr1000v",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for CiscoCsr1000vInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "gig1" => Ok(CiscoCsr1000vInt::Gig1),
            "gig2" => Ok(CiscoCsr1000vInt::Gig2),
            "gig3" => Ok(CiscoCsr1000vInt::Gig3),
            "gig4" => Ok(CiscoCsr1000vInt::Gig4),
            "gig5" => Ok(CiscoCsr1000vInt::Gig5),
            "gig6" => Ok(CiscoCsr1000vInt::Gig6),
            "gig7" => Ok(CiscoCsr1000vInt::Gig7),
            "gig8" => Ok(CiscoCsr1000vInt::Gig8),
            "gig9" => Ok(CiscoCsr1000vInt::Gig9),
            "gig10" => Ok(CiscoCsr1000vInt::Gig10),
            "gig11" => Ok(CiscoCsr1000vInt::Gig11),
            "gig12" => Ok(CiscoCsr1000vInt::Gig12),
            "gig13" => Ok(CiscoCsr1000vInt::Gig13),
            "gig14" => Ok(CiscoCsr1000vInt::Gig14),
            "gig15" => Ok(CiscoCsr1000vInt::Gig15),
            "gig16" => Ok(CiscoCsr1000vInt::Gig16),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "CiscoCsr1000v",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum CiscoCat8000vInt {
    Gig1,
    Gig2,
    Gig3,
    Gig4,
    Gig5,
    Gig6,
    Gig7,
    Gig8,
    Gig9,
    Gig10,
    Gig11,
    Gig12,
    Gig13,
    Gig14,
    Gig15,
    Gig16,
}
impl InterfaceTrait for CiscoCat8000vInt {
    fn to_idx(&self) -> u8 {
        match self {
            CiscoCat8000vInt::Gig1 => 1,
            CiscoCat8000vInt::Gig2 => 2,
            CiscoCat8000vInt::Gig3 => 3,
            CiscoCat8000vInt::Gig4 => 4,
            CiscoCat8000vInt::Gig5 => 5,
            CiscoCat8000vInt::Gig6 => 6,
            CiscoCat8000vInt::Gig7 => 7,
            CiscoCat8000vInt::Gig8 => 8,
            CiscoCat8000vInt::Gig9 => 9,
            CiscoCat8000vInt::Gig10 => 10,
            CiscoCat8000vInt::Gig11 => 11,
            CiscoCat8000vInt::Gig12 => 12,
            CiscoCat8000vInt::Gig13 => 13,
            CiscoCat8000vInt::Gig14 => 14,
            CiscoCat8000vInt::Gig15 => 15,
            CiscoCat8000vInt::Gig16 => 16,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            1..=16 => format!("gig{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "CiscoCat8000v",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for CiscoCat8000vInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "gig1" => Ok(CiscoCat8000vInt::Gig1),
            "gig2" => Ok(CiscoCat8000vInt::Gig2),
            "gig3" => Ok(CiscoCat8000vInt::Gig3),
            "gig4" => Ok(CiscoCat8000vInt::Gig4),
            "gig5" => Ok(CiscoCat8000vInt::Gig5),
            "gig6" => Ok(CiscoCat8000vInt::Gig6),
            "gig7" => Ok(CiscoCat8000vInt::Gig7),
            "gig8" => Ok(CiscoCat8000vInt::Gig8),
            "gig9" => Ok(CiscoCat8000vInt::Gig9),
            "gig10" => Ok(CiscoCat8000vInt::Gig10),
            "gig11" => Ok(CiscoCat8000vInt::Gig11),
            "gig12" => Ok(CiscoCat8000vInt::Gig12),
            "gig13" => Ok(CiscoCat8000vInt::Gig13),
            "gig14" => Ok(CiscoCat8000vInt::Gig14),
            "gig15" => Ok(CiscoCat8000vInt::Gig15),
            "gig16" => Ok(CiscoCat8000vInt::Gig16),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "CiscoCat8000v",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CiscoCat9000vInt {
    #[serde(rename = "gig0/0/1")]
    Gig0_0_1,
    #[serde(rename = "gig0/0/2")]
    Gig0_0_2,
    #[serde(rename = "gig0/0/3")]
    Gig0_0_3,
    #[serde(rename = "gig0/0/4")]
    Gig0_0_4,
    #[serde(rename = "gig0/0/5")]
    Gig0_0_5,
    #[serde(rename = "gig0/0/6")]
    Gig0_0_6,
    #[serde(rename = "gig0/0/7")]
    Gig0_0_7,
    #[serde(rename = "gig0/0/8")]
    Gig0_0_8,
}
impl InterfaceTrait for CiscoCat9000vInt {
    fn to_idx(&self) -> u8 {
        match self {
            CiscoCat9000vInt::Gig0_0_1 => 1,
            CiscoCat9000vInt::Gig0_0_2 => 2,
            CiscoCat9000vInt::Gig0_0_3 => 3,
            CiscoCat9000vInt::Gig0_0_4 => 4,
            CiscoCat9000vInt::Gig0_0_5 => 5,
            CiscoCat9000vInt::Gig0_0_6 => 6,
            CiscoCat9000vInt::Gig0_0_7 => 7,
            CiscoCat9000vInt::Gig0_0_8 => 8,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            1..=8 => format!("gig0/0/{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "CiscoCat9000v",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for CiscoCat9000vInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "gig0/0/1" => Ok(CiscoCat9000vInt::Gig0_0_1),
            "gig0/0/2" => Ok(CiscoCat9000vInt::Gig0_0_2),
            "gig0/0/3" => Ok(CiscoCat9000vInt::Gig0_0_3),
            "gig0/0/4" => Ok(CiscoCat9000vInt::Gig0_0_4),
            "gig0/0/5" => Ok(CiscoCat9000vInt::Gig0_0_5),
            "gig0/0/6" => Ok(CiscoCat9000vInt::Gig0_0_6),
            "gig0/0/7" => Ok(CiscoCat9000vInt::Gig0_0_7),
            "gig0/0/8" => Ok(CiscoCat9000vInt::Gig0_0_8),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "CiscoCat9000v",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CiscoNexus9300vInt {
    #[serde(rename = "eth1/1")]
    Eth1_1,
    #[serde(rename = "eth1/2")]
    Eth1_2,
    #[serde(rename = "eth1/3")]
    Eth1_3,
    #[serde(rename = "eth1/4")]
    Eth1_4,
    #[serde(rename = "eth1/5")]
    Eth1_5,
    #[serde(rename = "eth1/6")]
    Eth1_6,
    #[serde(rename = "eth1/7")]
    Eth1_7,
    #[serde(rename = "eth1/8")]
    Eth1_8,
    #[serde(rename = "eth1/9")]
    Eth1_9,
    #[serde(rename = "eth1/10")]
    Eth1_10,
    #[serde(rename = "eth1/11")]
    Eth1_11,
    #[serde(rename = "eth1/12")]
    Eth1_12,
    #[serde(rename = "eth1/13")]
    Eth1_13,
    #[serde(rename = "eth1/14")]
    Eth1_14,
    #[serde(rename = "eth1/15")]
    Eth1_15,
    #[serde(rename = "eth1/16")]
    Eth1_16,
    #[serde(rename = "eth1/17")]
    Eth1_17,
    #[serde(rename = "eth1/18")]
    Eth1_18,
    #[serde(rename = "eth1/19")]
    Eth1_19,
    #[serde(rename = "eth1/20")]
    Eth1_20,
    #[serde(rename = "eth1/21")]
    Eth1_21,
    #[serde(rename = "eth1/22")]
    Eth1_22,
    #[serde(rename = "eth1/23")]
    Eth1_23,
    #[serde(rename = "eth1/24")]
    Eth1_24,
    #[serde(rename = "eth1/25")]
    Eth1_25,
    #[serde(rename = "eth1/26")]
    Eth1_26,
    #[serde(rename = "eth1/27")]
    Eth1_27,
    #[serde(rename = "eth1/28")]
    Eth1_28,
    #[serde(rename = "eth1/29")]
    Eth1_29,
    #[serde(rename = "eth1/30")]
    Eth1_30,
    #[serde(rename = "eth1/31")]
    Eth1_31,
    #[serde(rename = "eth1/32")]
    Eth1_32,
    #[serde(rename = "eth1/33")]
    Eth1_33,
    #[serde(rename = "eth1/34")]
    Eth1_34,
    #[serde(rename = "eth1/35")]
    Eth1_35,
    #[serde(rename = "eth1/36")]
    Eth1_36,
    #[serde(rename = "eth1/37")]
    Eth1_37,
    #[serde(rename = "eth1/38")]
    Eth1_38,
    #[serde(rename = "eth1/39")]
    Eth1_39,
    #[serde(rename = "eth1/40")]
    Eth1_40,
    #[serde(rename = "eth1/41")]
    Eth1_41,
    #[serde(rename = "eth1/42")]
    Eth1_42,
    #[serde(rename = "eth1/43")]
    Eth1_43,
    #[serde(rename = "eth1/44")]
    Eth1_44,
    #[serde(rename = "eth1/45")]
    Eth1_45,
    #[serde(rename = "eth1/46")]
    Eth1_46,
    #[serde(rename = "eth1/47")]
    Eth1_47,
    #[serde(rename = "eth1/48")]
    Eth1_48,
    #[serde(rename = "eth1/49")]
    Eth1_49,
    #[serde(rename = "eth1/50")]
    Eth1_50,
    #[serde(rename = "eth1/51")]
    Eth1_51,
    #[serde(rename = "eth1/52")]
    Eth1_52,
    #[serde(rename = "eth1/53")]
    Eth1_53,
    #[serde(rename = "eth1/54")]
    Eth1_54,
    #[serde(rename = "eth1/55")]
    Eth1_55,
    #[serde(rename = "eth1/56")]
    Eth1_56,
    #[serde(rename = "eth1/57")]
    Eth1_57,
    #[serde(rename = "eth1/58")]
    Eth1_58,
    #[serde(rename = "eth1/59")]
    Eth1_59,
    #[serde(rename = "eth1/60")]
    Eth1_60,
    #[serde(rename = "eth1/61")]
    Eth1_61,
    #[serde(rename = "eth1/62")]
    Eth1_62,
    #[serde(rename = "eth1/63")]
    Eth1_63,
    #[serde(rename = "eth1/64")]
    Eth1_64,
}

impl InterfaceTrait for CiscoNexus9300vInt {
    fn to_idx(&self) -> u8 {
        match self {
            CiscoNexus9300vInt::Eth1_1 => 1,
            CiscoNexus9300vInt::Eth1_2 => 2,
            CiscoNexus9300vInt::Eth1_3 => 3,
            CiscoNexus9300vInt::Eth1_4 => 4,
            CiscoNexus9300vInt::Eth1_5 => 5,
            CiscoNexus9300vInt::Eth1_6 => 6,
            CiscoNexus9300vInt::Eth1_7 => 7,
            CiscoNexus9300vInt::Eth1_8 => 8,
            CiscoNexus9300vInt::Eth1_9 => 9,
            CiscoNexus9300vInt::Eth1_10 => 10,
            CiscoNexus9300vInt::Eth1_11 => 11,
            CiscoNexus9300vInt::Eth1_12 => 12,
            CiscoNexus9300vInt::Eth1_13 => 13,
            CiscoNexus9300vInt::Eth1_14 => 14,
            CiscoNexus9300vInt::Eth1_15 => 15,
            CiscoNexus9300vInt::Eth1_16 => 16,
            CiscoNexus9300vInt::Eth1_17 => 17,
            CiscoNexus9300vInt::Eth1_18 => 18,
            CiscoNexus9300vInt::Eth1_19 => 19,
            CiscoNexus9300vInt::Eth1_20 => 20,
            CiscoNexus9300vInt::Eth1_21 => 21,
            CiscoNexus9300vInt::Eth1_22 => 22,
            CiscoNexus9300vInt::Eth1_23 => 23,
            CiscoNexus9300vInt::Eth1_24 => 24,
            CiscoNexus9300vInt::Eth1_25 => 25,
            CiscoNexus9300vInt::Eth1_26 => 26,
            CiscoNexus9300vInt::Eth1_27 => 27,
            CiscoNexus9300vInt::Eth1_28 => 28,
            CiscoNexus9300vInt::Eth1_29 => 29,
            CiscoNexus9300vInt::Eth1_30 => 30,
            CiscoNexus9300vInt::Eth1_31 => 31,
            CiscoNexus9300vInt::Eth1_32 => 32,
            CiscoNexus9300vInt::Eth1_33 => 33,
            CiscoNexus9300vInt::Eth1_34 => 34,
            CiscoNexus9300vInt::Eth1_35 => 35,
            CiscoNexus9300vInt::Eth1_36 => 36,
            CiscoNexus9300vInt::Eth1_37 => 37,
            CiscoNexus9300vInt::Eth1_38 => 38,
            CiscoNexus9300vInt::Eth1_39 => 39,
            CiscoNexus9300vInt::Eth1_40 => 40,
            CiscoNexus9300vInt::Eth1_41 => 41,
            CiscoNexus9300vInt::Eth1_42 => 42,
            CiscoNexus9300vInt::Eth1_43 => 43,
            CiscoNexus9300vInt::Eth1_44 => 44,
            CiscoNexus9300vInt::Eth1_45 => 45,
            CiscoNexus9300vInt::Eth1_46 => 46,
            CiscoNexus9300vInt::Eth1_47 => 47,
            CiscoNexus9300vInt::Eth1_48 => 48,
            CiscoNexus9300vInt::Eth1_49 => 49,
            CiscoNexus9300vInt::Eth1_50 => 50,
            CiscoNexus9300vInt::Eth1_51 => 51,
            CiscoNexus9300vInt::Eth1_52 => 52,
            CiscoNexus9300vInt::Eth1_53 => 53,
            CiscoNexus9300vInt::Eth1_54 => 54,
            CiscoNexus9300vInt::Eth1_55 => 55,
            CiscoNexus9300vInt::Eth1_56 => 56,
            CiscoNexus9300vInt::Eth1_57 => 57,
            CiscoNexus9300vInt::Eth1_58 => 58,
            CiscoNexus9300vInt::Eth1_59 => 59,
            CiscoNexus9300vInt::Eth1_60 => 60,
            CiscoNexus9300vInt::Eth1_61 => 61,
            CiscoNexus9300vInt::Eth1_62 => 62,
            CiscoNexus9300vInt::Eth1_63 => 63,
            CiscoNexus9300vInt::Eth1_64 => 64,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            1..=64 => format!("eth1/{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "CiscoNexus9300v",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for CiscoNexus9300vInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "eth1/1" => Ok(CiscoNexus9300vInt::Eth1_1),
            "eth1/2" => Ok(CiscoNexus9300vInt::Eth1_2),
            "eth1/3" => Ok(CiscoNexus9300vInt::Eth1_3),
            "eth1/4" => Ok(CiscoNexus9300vInt::Eth1_4),
            "eth1/5" => Ok(CiscoNexus9300vInt::Eth1_5),
            "eth1/6" => Ok(CiscoNexus9300vInt::Eth1_6),
            "eth1/7" => Ok(CiscoNexus9300vInt::Eth1_7),
            "eth1/8" => Ok(CiscoNexus9300vInt::Eth1_8),
            "eth1/9" => Ok(CiscoNexus9300vInt::Eth1_9),
            "eth1/10" => Ok(CiscoNexus9300vInt::Eth1_10),
            "eth1/11" => Ok(CiscoNexus9300vInt::Eth1_11),
            "eth1/12" => Ok(CiscoNexus9300vInt::Eth1_12),
            "eth1/13" => Ok(CiscoNexus9300vInt::Eth1_13),
            "eth1/14" => Ok(CiscoNexus9300vInt::Eth1_14),
            "eth1/15" => Ok(CiscoNexus9300vInt::Eth1_15),
            "eth1/16" => Ok(CiscoNexus9300vInt::Eth1_16),
            "eth1/17" => Ok(CiscoNexus9300vInt::Eth1_17),
            "eth1/18" => Ok(CiscoNexus9300vInt::Eth1_18),
            "eth1/19" => Ok(CiscoNexus9300vInt::Eth1_19),
            "eth1/20" => Ok(CiscoNexus9300vInt::Eth1_20),
            "eth1/21" => Ok(CiscoNexus9300vInt::Eth1_21),
            "eth1/22" => Ok(CiscoNexus9300vInt::Eth1_22),
            "eth1/23" => Ok(CiscoNexus9300vInt::Eth1_23),
            "eth1/24" => Ok(CiscoNexus9300vInt::Eth1_24),
            "eth1/25" => Ok(CiscoNexus9300vInt::Eth1_25),
            "eth1/26" => Ok(CiscoNexus9300vInt::Eth1_26),
            "eth1/27" => Ok(CiscoNexus9300vInt::Eth1_27),
            "eth1/28" => Ok(CiscoNexus9300vInt::Eth1_28),
            "eth1/29" => Ok(CiscoNexus9300vInt::Eth1_29),
            "eth1/30" => Ok(CiscoNexus9300vInt::Eth1_30),
            "eth1/31" => Ok(CiscoNexus9300vInt::Eth1_31),
            "eth1/32" => Ok(CiscoNexus9300vInt::Eth1_32),
            "eth1/33" => Ok(CiscoNexus9300vInt::Eth1_33),
            "eth1/34" => Ok(CiscoNexus9300vInt::Eth1_34),
            "eth1/35" => Ok(CiscoNexus9300vInt::Eth1_35),
            "eth1/36" => Ok(CiscoNexus9300vInt::Eth1_36),
            "eth1/37" => Ok(CiscoNexus9300vInt::Eth1_37),
            "eth1/38" => Ok(CiscoNexus9300vInt::Eth1_38),
            "eth1/39" => Ok(CiscoNexus9300vInt::Eth1_39),
            "eth1/40" => Ok(CiscoNexus9300vInt::Eth1_40),
            "eth1/41" => Ok(CiscoNexus9300vInt::Eth1_41),
            "eth1/42" => Ok(CiscoNexus9300vInt::Eth1_42),
            "eth1/43" => Ok(CiscoNexus9300vInt::Eth1_43),
            "eth1/44" => Ok(CiscoNexus9300vInt::Eth1_44),
            "eth1/45" => Ok(CiscoNexus9300vInt::Eth1_45),
            "eth1/46" => Ok(CiscoNexus9300vInt::Eth1_46),
            "eth1/47" => Ok(CiscoNexus9300vInt::Eth1_47),
            "eth1/48" => Ok(CiscoNexus9300vInt::Eth1_48),
            "eth1/49" => Ok(CiscoNexus9300vInt::Eth1_49),
            "eth1/50" => Ok(CiscoNexus9300vInt::Eth1_50),
            "eth1/51" => Ok(CiscoNexus9300vInt::Eth1_51),
            "eth1/52" => Ok(CiscoNexus9300vInt::Eth1_52),
            "eth1/53" => Ok(CiscoNexus9300vInt::Eth1_53),
            "eth1/54" => Ok(CiscoNexus9300vInt::Eth1_54),
            "eth1/55" => Ok(CiscoNexus9300vInt::Eth1_55),
            "eth1/56" => Ok(CiscoNexus9300vInt::Eth1_56),
            "eth1/57" => Ok(CiscoNexus9300vInt::Eth1_57),
            "eth1/58" => Ok(CiscoNexus9300vInt::Eth1_58),
            "eth1/59" => Ok(CiscoNexus9300vInt::Eth1_59),
            "eth1/60" => Ok(CiscoNexus9300vInt::Eth1_60),
            "eth1/61" => Ok(CiscoNexus9300vInt::Eth1_61),
            "eth1/62" => Ok(CiscoNexus9300vInt::Eth1_62),
            "eth1/63" => Ok(CiscoNexus9300vInt::Eth1_63),
            "eth1/64" => Ok(CiscoNexus9300vInt::Eth1_64),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "CiscoNexus9300v",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum JuniperVrouterInt {
    #[serde(rename = "ge-0/0/0")]
    Ge0_0_0,
    #[serde(rename = "ge-0/0/1")]
    Ge0_0_1,
    #[serde(rename = "ge-0/0/2")]
    Ge0_0_2,
    #[serde(rename = "ge-0/0/3")]
    Ge0_0_3,
    #[serde(rename = "ge-0/0/4")]
    Ge0_0_4,
    #[serde(rename = "ge-0/0/5")]
    Ge0_0_5,
    #[serde(rename = "ge-0/0/6")]
    Ge0_0_6,
    #[serde(rename = "ge-0/0/7")]
    Ge0_0_7,
    #[serde(rename = "ge-0/0/8")]
    Ge0_0_8,
    #[serde(rename = "ge-0/0/9")]
    Ge0_0_9,
}
impl InterfaceTrait for JuniperVrouterInt {
    fn to_idx(&self) -> u8 {
        match self {
            JuniperVrouterInt::Ge0_0_0 => 0,
            JuniperVrouterInt::Ge0_0_1 => 1,
            JuniperVrouterInt::Ge0_0_2 => 2,
            JuniperVrouterInt::Ge0_0_3 => 3,
            JuniperVrouterInt::Ge0_0_4 => 4,
            JuniperVrouterInt::Ge0_0_5 => 5,
            JuniperVrouterInt::Ge0_0_6 => 6,
            JuniperVrouterInt::Ge0_0_7 => 7,
            JuniperVrouterInt::Ge0_0_8 => 8,
            JuniperVrouterInt::Ge0_0_9 => 9,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            0..=9 => format!("ge-0/0/{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "JuniperVrouter",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for JuniperVrouterInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "ge-0/0/0" => Ok(JuniperVrouterInt::Ge0_0_0),
            "ge-0/0/1" => Ok(JuniperVrouterInt::Ge0_0_1),
            "ge-0/0/2" => Ok(JuniperVrouterInt::Ge0_0_2),
            "ge-0/0/3" => Ok(JuniperVrouterInt::Ge0_0_3),
            "ge-0/0/4" => Ok(JuniperVrouterInt::Ge0_0_4),
            "ge-0/0/5" => Ok(JuniperVrouterInt::Ge0_0_5),
            "ge-0/0/6" => Ok(JuniperVrouterInt::Ge0_0_6),
            "ge-0/0/7" => Ok(JuniperVrouterInt::Ge0_0_7),
            "ge-0/0/8" => Ok(JuniperVrouterInt::Ge0_0_8),
            "ge-0/0/9" => Ok(JuniperVrouterInt::Ge0_0_9),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "JuniperVrouter",
                iface: text.to_string(),
            }),
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum JuniperVswitchInt {
    #[serde(rename = "ge-0/0/0")]
    Ge0_0_0,
    #[serde(rename = "ge-0/0/1")]
    Ge0_0_1,
    #[serde(rename = "ge-0/0/2")]
    Ge0_0_2,
    #[serde(rename = "ge-0/0/3")]
    Ge0_0_3,
    #[serde(rename = "ge-0/0/4")]
    Ge0_0_4,
    #[serde(rename = "ge-0/0/5")]
    Ge0_0_5,
    #[serde(rename = "ge-0/0/6")]
    Ge0_0_6,
    #[serde(rename = "ge-0/0/7")]
    Ge0_0_7,
    #[serde(rename = "ge-0/0/8")]
    Ge0_0_8,
    #[serde(rename = "ge-0/0/9")]
    Ge0_0_9,
}
impl InterfaceTrait for JuniperVswitchInt {
    fn to_idx(&self) -> u8 {
        match self {
            JuniperVswitchInt::Ge0_0_0 => 0,
            JuniperVswitchInt::Ge0_0_1 => 1,
            JuniperVswitchInt::Ge0_0_2 => 2,
            JuniperVswitchInt::Ge0_0_3 => 3,
            JuniperVswitchInt::Ge0_0_4 => 4,
            JuniperVswitchInt::Ge0_0_5 => 5,
            JuniperVswitchInt::Ge0_0_6 => 6,
            JuniperVswitchInt::Ge0_0_7 => 7,
            JuniperVswitchInt::Ge0_0_8 => 8,
            JuniperVswitchInt::Ge0_0_9 => 9,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            0..=9 => format!("ge-0/0/{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "JuniperVswitch",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for JuniperVswitchInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "ge-0/0/0" => Ok(JuniperVswitchInt::Ge0_0_0),
            "ge-0/0/1" => Ok(JuniperVswitchInt::Ge0_0_1),
            "ge-0/0/2" => Ok(JuniperVswitchInt::Ge0_0_2),
            "ge-0/0/3" => Ok(JuniperVswitchInt::Ge0_0_3),
            "ge-0/0/4" => Ok(JuniperVswitchInt::Ge0_0_4),
            "ge-0/0/5" => Ok(JuniperVswitchInt::Ge0_0_5),
            "ge-0/0/6" => Ok(JuniperVswitchInt::Ge0_0_6),
            "ge-0/0/7" => Ok(JuniperVswitchInt::Ge0_0_7),
            "ge-0/0/8" => Ok(JuniperVswitchInt::Ge0_0_8),
            "ge-0/0/9" => Ok(JuniperVswitchInt::Ge0_0_9),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "JuniperVswitch",
                iface: text.to_string(),
            }),
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum JuniperVevolvedInt {
    #[serde(rename = "et-0/0/0")]
    Et0_0_0,
    #[serde(rename = "et-0/0/1")]
    Et0_0_1,
    #[serde(rename = "et-0/0/2")]
    Et0_0_2,
    #[serde(rename = "et-0/0/3")]
    Et0_0_3,
    #[serde(rename = "et-0/0/4")]
    Et0_0_4,
    #[serde(rename = "et-0/0/5")]
    Et0_0_5,
    #[serde(rename = "et-0/0/6")]
    Et0_0_6,
    #[serde(rename = "et-0/0/7")]
    Et0_0_7,
    #[serde(rename = "et-0/0/8")]
    Et0_0_8,
    #[serde(rename = "et-0/0/9")]
    Et0_0_9,
    #[serde(rename = "et-0/0/10")]
    Et0_0_10,
    #[serde(rename = "et-0/0/11")]
    Et0_0_11,
}
impl InterfaceTrait for JuniperVevolvedInt {
    fn to_idx(&self) -> u8 {
        match self {
            JuniperVevolvedInt::Et0_0_0 => 0,
            JuniperVevolvedInt::Et0_0_1 => 1,
            JuniperVevolvedInt::Et0_0_2 => 2,
            JuniperVevolvedInt::Et0_0_3 => 3,
            JuniperVevolvedInt::Et0_0_4 => 4,
            JuniperVevolvedInt::Et0_0_5 => 5,
            JuniperVevolvedInt::Et0_0_6 => 6,
            JuniperVevolvedInt::Et0_0_7 => 7,
            JuniperVevolvedInt::Et0_0_8 => 8,
            JuniperVevolvedInt::Et0_0_9 => 9,
            JuniperVevolvedInt::Et0_0_10 => 10,
            JuniperVevolvedInt::Et0_0_11 => 11,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            0..=11 => format!("et-0/0/{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "JuniperVevolved",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for JuniperVevolvedInt {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "et-0/0/0" => Ok(JuniperVevolvedInt::Et0_0_0),
            "et-0/0/1" => Ok(JuniperVevolvedInt::Et0_0_1),
            "et-0/0/2" => Ok(JuniperVevolvedInt::Et0_0_2),
            "et-0/0/3" => Ok(JuniperVevolvedInt::Et0_0_3),
            "et-0/0/4" => Ok(JuniperVevolvedInt::Et0_0_4),
            "et-0/0/5" => Ok(JuniperVevolvedInt::Et0_0_5),
            "et-0/0/6" => Ok(JuniperVevolvedInt::Et0_0_6),
            "et-0/0/7" => Ok(JuniperVevolvedInt::Et0_0_7),
            "et-0/0/8" => Ok(JuniperVevolvedInt::Et0_0_8),
            "et-0/0/9" => Ok(JuniperVevolvedInt::Et0_0_9),
            "et-0/0/10" => Ok(JuniperVevolvedInt::Et0_0_10),
            "et-0/0/11" => Ok(JuniperVevolvedInt::Et0_0_11),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "JuniperVevolved",
                iface: text.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum JuniperVsrxv3Int {
    #[serde(rename = "ge-0/0/0")]
    Ge0_0_0,
    #[serde(rename = "ge-0/0/1")]
    Ge0_0_1,
    #[serde(rename = "ge-0/0/2")]
    Ge0_0_2,
    #[serde(rename = "ge-0/0/3")]
    Ge0_0_3,
    #[serde(rename = "ge-0/0/4")]
    Ge0_0_4,
    #[serde(rename = "ge-0/0/5")]
    Ge0_0_5,
    #[serde(rename = "ge-0/0/6")]
    Ge0_0_6,
    #[serde(rename = "ge-0/0/7")]
    Ge0_0_7,
}
impl InterfaceTrait for JuniperVsrxv3Int {
    fn to_idx(&self) -> u8 {
        match self {
            JuniperVsrxv3Int::Ge0_0_0 => 0,
            JuniperVsrxv3Int::Ge0_0_1 => 1,
            JuniperVsrxv3Int::Ge0_0_2 => 2,
            JuniperVsrxv3Int::Ge0_0_3 => 3,
            JuniperVsrxv3Int::Ge0_0_4 => 4,
            JuniperVsrxv3Int::Ge0_0_5 => 5,
            JuniperVsrxv3Int::Ge0_0_6 => 6,
            JuniperVsrxv3Int::Ge0_0_7 => 7,
        }
    }
    fn from_idx(idx: u8) -> Result<String, ParseInterfaceIdxError> {
        let iface = match idx {
            0..=7 => format!("ge-0/0/{}", idx),
            _ => String::new(),
        };
        if iface.is_empty() {
            Err(ParseInterfaceIdxError::UnknownInterfaceIdx {
                enum_name: "JuniperVsrxv3",
                idx,
            })
        } else {
            Ok(iface)
        }
    }
}
impl FromStr for JuniperVsrxv3Int {
    type Err = ParseInterfaceStrError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "ge-0/0/0" => Ok(JuniperVsrxv3Int::Ge0_0_0),
            "ge-0/0/1" => Ok(JuniperVsrxv3Int::Ge0_0_1),
            "ge-0/0/2" => Ok(JuniperVsrxv3Int::Ge0_0_2),
            "ge-0/0/3" => Ok(JuniperVsrxv3Int::Ge0_0_3),
            "ge-0/0/4" => Ok(JuniperVsrxv3Int::Ge0_0_4),
            "ge-0/0/5" => Ok(JuniperVsrxv3Int::Ge0_0_5),
            "ge-0/0/6" => Ok(JuniperVsrxv3Int::Ge0_0_6),
            "ge-0/0/7" => Ok(JuniperVsrxv3Int::Ge0_0_7),
            _ => Err(ParseInterfaceStrError::UnknownInterfaceStr {
                enum_name: "JuniperVsrxv3",
                iface: text.to_string(),
            }),
        }
    }
}
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
