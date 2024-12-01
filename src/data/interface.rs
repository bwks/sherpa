use std::fmt;

use serde_derive::{Deserialize, Serialize};

use crate::data::InterfaceConnection;

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
#[allow(dead_code)]
pub enum MgmtInterfaces {
    #[default]
    #[serde(rename(serialize = "eth0", deserialize = "eth0"))]
    Eth0, // eth0 - cumulus-vx, linux
    #[serde(rename(serialize = "GigabitEthernet0/0", deserialize = "GigabitEthernet0/0"))]
    GigabitEthernet0_0, // GigabitEthernet0/0 - cat9k, iosv/l2
    #[serde(rename(serialize = "GigabitEthernet1", deserialize = "GigabitEthernet1"))]
    GigabitEthernet1, // GigabitEthernet1 - cat1/8k
    #[serde(rename(serialize = "fxp0", deserialize = "fxp0"))]
    Fxp0, // Fxp0 - Junos
    #[serde(rename(serialize = "mgmt", deserialize = "mgmt"))]
    Mgmt, // mgmt - aos
    #[serde(rename(serialize = "mgmt0", deserialize = "mgmt0"))]
    Mgmt0, // mgmt0 - n93kv
    #[serde(rename(serialize = "Management0/0", deserialize = "Management0/0"))]
    Management0_0, // Management0/0 - asav
    #[serde(rename(serialize = "Management1", deserialize = "Management1"))]
    Management1, // Management1 - eos
    #[serde(rename(serialize = "MgmtEth0/RP0/CPU0/0", deserialize = "MgmtEth0/RP0/CPU0/0"))]
    MgmtEth0Rp0Cpu0_0, // MgmtEth0/RP0/CPU0/0 - xr9kv
    #[serde(rename(serialize = "Vlan1", deserialize = "Vlan1"))]
    Vlan1, // Vlan1 - iosvl2
}
impl fmt::Display for MgmtInterfaces {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MgmtInterfaces::Eth0 => write!(f, "eth0"),
            MgmtInterfaces::GigabitEthernet1 => write!(f, "GigabitEthernet1"),
            MgmtInterfaces::GigabitEthernet0_0 => write!(f, "GigabitEthernet0/0"),
            MgmtInterfaces::Fxp0 => write!(f, "fxp0"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_test::{assert_tokens, Token};

    #[test]
    fn test_mgmt_interfaces_serialization() {
        // Test Eth0 variant
        assert_tokens(
            &MgmtInterfaces::Eth0,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "eth0",
            }],
        );

        // Test Mgmt variant
        assert_tokens(
            &MgmtInterfaces::Mgmt,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "mgmt",
            }],
        );
        // Test Mgmt0 variant
        assert_tokens(
            &MgmtInterfaces::Mgmt0,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "mgmt0",
            }],
        );
        // Test Management1 variant
        assert_tokens(
            &MgmtInterfaces::Management1,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "Management1",
            }],
        );
        // Test Management0/0 variant
        assert_tokens(
            &MgmtInterfaces::Management0_0,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "Management0/0",
            }],
        );
        // Test GigabitEthernet1 variant
        assert_tokens(
            &MgmtInterfaces::GigabitEthernet1,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "GigabitEthernet1",
            }],
        );
        // Test GigabitEthernet0/0 variant
        assert_tokens(
            &MgmtInterfaces::GigabitEthernet0_0,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "GigabitEthernet0/0",
            }],
        );
        // Test MgmtEth0/RP0/CPU0/0 variant
        assert_tokens(
            &MgmtInterfaces::MgmtEth0Rp0Cpu0_0,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "MgmtEth0/RP0/CPU0/0",
            }],
        );
        // Test fxp0 variant
        assert_tokens(
            &MgmtInterfaces::Fxp0,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "fxp0",
            }],
        );
        // Test Vlan1 variant
        assert_tokens(
            &MgmtInterfaces::Vlan1,
            &[Token::UnitVariant {
                name: "MgmtInterfaces",
                variant: "Vlan1",
            }],
        );
    }

    #[test]
    fn test_mgmt_interfaces_deserialization() {
        // Test string to enum conversion
        let eth0: MgmtInterfaces = serde_json::from_str(r#""eth0""#).unwrap();
        assert!(matches!(eth0, MgmtInterfaces::Eth0));

        let mgmt: MgmtInterfaces = serde_json::from_str(r#""mgmt""#).unwrap();
        assert!(matches!(mgmt, MgmtInterfaces::Mgmt));

        let mgmt0: MgmtInterfaces = serde_json::from_str(r#""mgmt0""#).unwrap();
        assert!(matches!(mgmt0, MgmtInterfaces::Mgmt0));

        let management1: MgmtInterfaces = serde_json::from_str(r#""Management1""#).unwrap();
        assert!(matches!(management1, MgmtInterfaces::Management1));

        let management0_0: MgmtInterfaces = serde_json::from_str(r#""Management0/0""#).unwrap();
        assert!(matches!(management0_0, MgmtInterfaces::Management0_0));

        let gigabit_ethernet1: MgmtInterfaces =
            serde_json::from_str(r#""GigabitEthernet1""#).unwrap();
        assert!(matches!(
            gigabit_ethernet1,
            MgmtInterfaces::GigabitEthernet1
        ));

        let gigabit_ethernet0_0: MgmtInterfaces =
            serde_json::from_str(r#""GigabitEthernet0/0""#).unwrap();
        assert!(matches!(
            gigabit_ethernet0_0,
            MgmtInterfaces::GigabitEthernet0_0
        ));

        let mgmteth0rp0cpu0_0: MgmtInterfaces =
            serde_json::from_str(r#""MgmtEth0/RP0/CPU0/0""#).unwrap();
        assert!(matches!(
            mgmteth0rp0cpu0_0,
            MgmtInterfaces::MgmtEth0Rp0Cpu0_0
        ));

        let fxp0: MgmtInterfaces = serde_json::from_str(r#""fxp0""#).unwrap();
        assert!(matches!(fxp0, MgmtInterfaces::Fxp0));

        let vlan1: MgmtInterfaces = serde_json::from_str(r#""Vlan1""#).unwrap();
        assert!(matches!(vlan1, MgmtInterfaces::Vlan1));
    }

    #[test]
    fn test_mgmt_interfaces_deserialization_error() {
        let result: Result<MgmtInterfaces, _> = serde_json::from_str(r#""invalid""#);
        assert!(result.is_err());
    }
}
