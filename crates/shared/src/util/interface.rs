use std::str::FromStr;

use anyhow::{Result, bail};

use crate::data::{
    AristaCeosInt, AristaVeosInt, ArubaAoscxInt, CiscoAsavInt, CiscoCat8000vInt, CiscoCat9000vInt,
    CiscoCsr1000vInt, CiscoFtdvInt, CiscoIosvInt, CiscoIosvl2Int, CiscoIosxrv9000Int,
    CiscoNexus9300vInt, CumulusLinuxInt, EthernetInt, InterfaceTrait, JuniperVevolvedInt,
    JuniperVrouterInt, JuniperVsrxv3Int, JuniperVswitchInt, MikrotikChrInt, NodeModel,
    NokiaSrlinuxInt, PaloaltoPanosInt,
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
        NodeModel::PaloaltoPanos => PaloaltoPanosInt::from_str(interface)?.to_idx(),
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
        NodeModel::PaloaltoPanos => PaloaltoPanosInt::from_idx(idx)?,
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
        NodeModel::PaloaltoPanos => PaloaltoPanosInt::all_interfaces(),
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

    #[test]
    fn test_interface_roundtrip_cisco_iosv() {
        let model = NodeModel::CiscoIosv;
        let name = interface_from_idx(&model, 0).unwrap();
        let idx = interface_to_idx(&model, &name).unwrap();
        assert_eq!(idx, 0);

        let name1 = interface_from_idx(&model, 1).unwrap();
        let idx1 = interface_to_idx(&model, &name1).unwrap();
        assert_eq!(idx1, 1);
    }

    #[test]
    fn test_interface_roundtrip_arista_veos() {
        let model = NodeModel::AristaVeos;
        for idx in 0..5 {
            let name = interface_from_idx(&model, idx).unwrap();
            let back = interface_to_idx(&model, &name).unwrap();
            assert_eq!(back, idx, "round-trip failed for idx {idx}: name={name}");
        }
    }

    #[test]
    fn test_interface_roundtrip_generic_ethernet() {
        let model = NodeModel::UbuntuLinux;
        let name = interface_from_idx(&model, 0).unwrap();
        assert_eq!(interface_to_idx(&model, &name).unwrap(), 0);
        let name3 = interface_from_idx(&model, 3).unwrap();
        assert_eq!(interface_to_idx(&model, &name3).unwrap(), 3);
    }

    #[test]
    fn test_node_model_interfaces_returns_nonempty() {
        assert!(!node_model_interfaces(&NodeModel::CiscoIosv).is_empty());
        assert!(!node_model_interfaces(&NodeModel::NokiaSrlinux).is_empty());
        assert!(!node_model_interfaces(&NodeModel::UbuntuLinux).is_empty());
    }

    // Helper: verify idx→name→idx round-trips correctly for a given model.
    fn assert_roundtrip(model: &NodeModel) {
        let name = interface_from_idx(model, 0).unwrap();
        let back = interface_to_idx(model, &name).unwrap();
        assert_eq!(
            back, 0,
            "{model:?}: round-trip failed for idx 0 (name={name})"
        );
    }

    #[test]
    fn test_roundtrip_arista_ceos() {
        assert_roundtrip(&NodeModel::AristaCeos);
    }

    #[test]
    fn test_roundtrip_aruba_aoscx() {
        assert_roundtrip(&NodeModel::ArubaAoscx);
    }

    #[test]
    fn test_roundtrip_cisco_asav() {
        assert_roundtrip(&NodeModel::CiscoAsav);
    }

    #[test]
    fn test_roundtrip_cisco_csr1000v() {
        assert_roundtrip(&NodeModel::CiscoCsr1000v);
    }

    #[test]
    fn test_roundtrip_cisco_cat8000v() {
        assert_roundtrip(&NodeModel::CiscoCat8000v);
    }

    #[test]
    fn test_roundtrip_cisco_cat9000v() {
        assert_roundtrip(&NodeModel::CiscoCat9000v);
    }

    #[test]
    fn test_roundtrip_cisco_iosxrv9000() {
        assert_roundtrip(&NodeModel::CiscoIosxrv9000);
    }

    #[test]
    fn test_roundtrip_cisco_nexus9300v() {
        assert_roundtrip(&NodeModel::CiscoNexus9300v);
    }

    #[test]
    fn test_roundtrip_cisco_iosvl2() {
        assert_roundtrip(&NodeModel::CiscoIosvl2);
    }

    #[test]
    fn test_roundtrip_cisco_ftdv() {
        assert_roundtrip(&NodeModel::CiscoFtdv);
    }

    #[test]
    fn test_roundtrip_juniper_vrouter() {
        assert_roundtrip(&NodeModel::JuniperVrouter);
    }

    #[test]
    fn test_roundtrip_juniper_vswitch() {
        assert_roundtrip(&NodeModel::JuniperVswitch);
    }

    #[test]
    fn test_roundtrip_juniper_vevolved() {
        assert_roundtrip(&NodeModel::JuniperVevolved);
    }

    #[test]
    fn test_roundtrip_juniper_vsrxv3() {
        assert_roundtrip(&NodeModel::JuniperVsrxv3);
    }

    #[test]
    fn test_roundtrip_cumulus_linux() {
        assert_roundtrip(&NodeModel::CumulusLinux);
    }

    #[test]
    fn test_roundtrip_nokia_srlinux() {
        assert_roundtrip(&NodeModel::NokiaSrlinux);
    }

    #[test]
    fn test_roundtrip_mikrotik_chr() {
        assert_roundtrip(&NodeModel::MikrotikChr);
    }

    #[test]
    fn test_roundtrip_paloalto_panos() {
        assert_roundtrip(&NodeModel::PaloaltoPanos);
    }

    #[test]
    fn test_node_model_interfaces_all_named_models() {
        let named_models = [
            NodeModel::AristaVeos,
            NodeModel::AristaCeos,
            NodeModel::ArubaAoscx,
            NodeModel::CiscoAsav,
            NodeModel::CiscoCsr1000v,
            NodeModel::CiscoCat8000v,
            NodeModel::CiscoCat9000v,
            NodeModel::CiscoIosxrv9000,
            NodeModel::CiscoNexus9300v,
            NodeModel::CiscoIosv,
            NodeModel::CiscoIosvl2,
            NodeModel::CiscoFtdv,
            NodeModel::JuniperVrouter,
            NodeModel::JuniperVswitch,
            NodeModel::JuniperVevolved,
            NodeModel::JuniperVsrxv3,
            NodeModel::CumulusLinux,
            NodeModel::NokiaSrlinux,
            NodeModel::MikrotikChr,
            NodeModel::PaloaltoPanos,
        ];
        for model in &named_models {
            let ifaces = node_model_interfaces(model);
            assert!(
                !ifaces.is_empty(),
                "{model:?} returned empty interface list"
            );
        }
    }
}
