use std::str::FromStr;

use anyhow::{Result, bail};

use crate::data::{
    AristaCeosInt, AristaVeosInt, ArubaAoscxInt, CiscoAsavInt, CiscoCat8000vInt, CiscoCat9000vInt,
    CiscoCsr1000vInt, CiscoFtdvInt, CiscoIosvInt, CiscoIosvl2Int, CiscoIosxrv9000Int,
    CiscoNexus9300vInt, CumulusLinuxInt, EthernetInt, InterfaceTrait, JuniperVevolvedInt,
    JuniperVrouterInt, JuniperVsrxv3Int, JuniperVswitchInt, MikrotikChrInt, NodeModel,
    NokiaSrlinuxInt,
};

pub fn interface_to_idx(device_model: &NodeModel, interface: &str) -> Result<u8> {
    let idx = match device_model {
        NodeModel::AristaVeos => AristaVeosInt::from_str(interface)?.to_idx(),
        NodeModel::AristaCeos => AristaCeosInt::from_str(interface)?.to_idx(),
        NodeModel::ArubaAoscx => ArubaAoscxInt::from_str(interface)?.to_idx(),
        NodeModel::CiscoAsav => CiscoAsavInt::from_str(interface)?.to_idx(),
        NodeModel::CiscoCsr1000v => CiscoCsr1000vInt::from_str(interface)?.to_idx(),
        NodeModel::CiscoCat8000v => CiscoCat8000vInt::from_str(interface)?.to_idx(),
        NodeModel::CiscoCat9000v => CiscoCat9000vInt::from_str(interface)?.to_idx(),
        NodeModel::CiscoIosxrv9000 => CiscoIosxrv9000Int::from_str(interface)?.to_idx(),
        NodeModel::CiscoNexus9300v => CiscoNexus9300vInt::from_str(interface)?.to_idx(),
        NodeModel::CiscoIosv => CiscoIosvInt::from_str(interface)?.to_idx(),
        NodeModel::CiscoIosvl2 => CiscoIosvl2Int::from_str(interface)?.to_idx(),
        NodeModel::CiscoFtdv => CiscoFtdvInt::from_str(interface)?.to_idx(),
        NodeModel::JuniperVrouter => JuniperVrouterInt::from_str(interface)?.to_idx(),
        NodeModel::JuniperVswitch => JuniperVswitchInt::from_str(interface)?.to_idx(),
        NodeModel::JuniperVevolved => JuniperVevolvedInt::from_str(interface)?.to_idx(),
        NodeModel::JuniperVsrxv3 => JuniperVsrxv3Int::from_str(interface)?.to_idx(),
        NodeModel::CumulusLinux => CumulusLinuxInt::from_str(interface)?.to_idx(),
        NodeModel::NokiaSrlinux => NokiaSrlinuxInt::from_str(interface)?.to_idx(),
        NodeModel::MikrotikChr => MikrotikChrInt::from_str(interface)?.to_idx(),
        _ => EthernetInt::from_str(interface)?.to_idx(),
    };
    Ok(idx)
}

pub fn interface_from_idx(device_model: &NodeModel, idx: u8) -> Result<String> {
    let iface = match device_model {
        NodeModel::AristaVeos => AristaVeosInt::from_idx(idx)?,
        NodeModel::AristaCeos => AristaCeosInt::from_idx(idx)?,
        NodeModel::ArubaAoscx => ArubaAoscxInt::from_idx(idx)?,
        NodeModel::CiscoAsav => CiscoAsavInt::from_idx(idx)?,
        NodeModel::CiscoCsr1000v => CiscoCsr1000vInt::from_idx(idx)?,
        NodeModel::CiscoCat8000v => CiscoCat8000vInt::from_idx(idx)?,
        NodeModel::CiscoCat9000v => CiscoCat9000vInt::from_idx(idx)?,
        NodeModel::CiscoIosxrv9000 => CiscoIosxrv9000Int::from_idx(idx)?,
        NodeModel::CiscoNexus9300v => CiscoNexus9300vInt::from_idx(idx)?,
        NodeModel::CiscoIosv => CiscoIosvInt::from_idx(idx)?,
        NodeModel::CiscoIosvl2 => CiscoIosvl2Int::from_idx(idx)?,
        NodeModel::CiscoFtdv => CiscoFtdvInt::from_idx(idx)?,
        NodeModel::JuniperVrouter => JuniperVrouterInt::from_idx(idx)?,
        NodeModel::JuniperVswitch => JuniperVswitchInt::from_idx(idx)?,
        NodeModel::JuniperVevolved => JuniperVevolvedInt::from_idx(idx)?,
        NodeModel::JuniperVsrxv3 => JuniperVsrxv3Int::from_idx(idx)?,
        NodeModel::CumulusLinux => CumulusLinuxInt::from_idx(idx)?,
        NodeModel::NokiaSrlinux => NokiaSrlinuxInt::from_idx(idx)?,
        NodeModel::MikrotikChr => MikrotikChrInt::from_idx(idx)?,
        _ => EthernetInt::from_idx(idx)?,
    };
    Ok(iface)
}

pub fn node_model_interfaces(device_model: &NodeModel) -> Vec<String> {
    match device_model {
        NodeModel::AristaVeos => AristaVeosInt::all_interfaces(),
        NodeModel::AristaCeos => AristaCeosInt::all_interfaces(),
        NodeModel::ArubaAoscx => ArubaAoscxInt::all_interfaces(),
        NodeModel::CiscoAsav => CiscoAsavInt::all_interfaces(),
        NodeModel::CiscoCsr1000v => CiscoCsr1000vInt::all_interfaces(),
        NodeModel::CiscoCat8000v => CiscoCat8000vInt::all_interfaces(),
        NodeModel::CiscoCat9000v => CiscoCat9000vInt::all_interfaces(),
        NodeModel::CiscoIosxrv9000 => CiscoIosxrv9000Int::all_interfaces(),
        NodeModel::CiscoNexus9300v => CiscoNexus9300vInt::all_interfaces(),
        NodeModel::CiscoIosv => CiscoIosvInt::all_interfaces(),
        NodeModel::CiscoIosvl2 => CiscoIosvl2Int::all_interfaces(),
        NodeModel::CiscoFtdv => CiscoFtdvInt::all_interfaces(),
        NodeModel::JuniperVrouter => JuniperVrouterInt::all_interfaces(),
        NodeModel::JuniperVswitch => JuniperVswitchInt::all_interfaces(),
        NodeModel::JuniperVevolved => JuniperVevolvedInt::all_interfaces(),
        NodeModel::JuniperVsrxv3 => JuniperVsrxv3Int::all_interfaces(),
        NodeModel::CumulusLinux => CumulusLinuxInt::all_interfaces(),
        NodeModel::NokiaSrlinux => NokiaSrlinuxInt::all_interfaces(),
        NodeModel::MikrotikChr => MikrotikChrInt::all_interfaces(),
        _ => EthernetInt::all_interfaces(),
    }
}

/// Convert an SR Linux interface name to the Linux-compatible name used
/// inside the container.  e.g. "eth-1/3" → "e1-3".
pub fn srlinux_to_linux_interface(name: &str) -> Result<String> {
    let rest = name.strip_prefix("eth-").unwrap_or(name);
    let linux_name = rest.replace('/', "-");
    let linux_name = format!("e{linux_name}");
    if linux_name == format!("e{name}") && !name.starts_with("eth-") {
        bail!("Not a recognised SR Linux interface name: {name}");
    }
    Ok(linux_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srlinux_to_linux_interface() {
        assert_eq!(srlinux_to_linux_interface("eth-1/1").unwrap(), "e1-1");
        assert_eq!(srlinux_to_linux_interface("eth-1/3").unwrap(), "e1-3");
        assert_eq!(srlinux_to_linux_interface("eth-1/34").unwrap(), "e1-34");
    }

    #[test]
    fn test_srlinux_to_linux_interface_invalid() {
        assert!(srlinux_to_linux_interface("mgmt0").is_err());
    }
}
