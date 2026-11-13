use std::str::FromStr;

use anyhow::{Result, bail};

use crate::data::{
    AristaCeosInt, AristaVeosInt, ArubaAoscxInt, CiscoAsavInt, CiscoCat8000vInt, CiscoCat9000vInt,
    CiscoCsr1000vInt, CiscoFtdvInt, CiscoIolInt, CiscoIosvInt, CiscoIosvl2Int, CiscoIosxrv9000Int,
    CiscoNexus9300vInt, CumulusLinuxInt, EthernetInt, InterfaceTrait, JuniperCevolvedInt,
    JuniperVevolvedInt, JuniperVrouterInt, JuniperVsrxv3Int, JuniperVswitchInt, MikrotikChrInt,
    NodeModel, NokiaSrlinuxInt, PaloaltoPanosInt,
};

/// The interfaces cJunosEvolved creates for itself in its container, `eth1` to `eth3`.
const CEVOLVED_INTERNAL_INTERFACES: u16 = 3;

/// The index of the interface named `interface` on `device_model`.
///
/// # Errors
///
/// Fails when the model has no interface named `interface`.
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
        NodeModel::CiscoIol | NodeModel::CiscoIolL2 => CiscoIolInt::from_str(interface)?.to_idx(),
        NodeModel::CiscoFtdv => CiscoFtdvInt::from_str(interface)?.to_idx(),
        NodeModel::JuniperVrouter => JuniperVrouterInt::from_str(interface)?.to_idx(),
        NodeModel::JuniperVswitch => JuniperVswitchInt::from_str(interface)?.to_idx(),
        NodeModel::JuniperVevolved => JuniperVevolvedInt::from_str(interface)?.to_idx(),
        NodeModel::JuniperCevolved => JuniperCevolvedInt::from_str(interface)?.to_idx(),
        NodeModel::JuniperVsrxv3 => JuniperVsrxv3Int::from_str(interface)?.to_idx(),
        NodeModel::CumulusLinux => CumulusLinuxInt::from_str(interface)?.to_idx(),
        NodeModel::NokiaSrlinux => NokiaSrlinuxInt::from_str(interface)?.to_idx(),
        NodeModel::MikrotikChr => MikrotikChrInt::from_str(interface)?.to_idx(),
        NodeModel::PaloaltoPanos => PaloaltoPanosInt::from_str(interface)?.to_idx(),
        _ => EthernetInt::from_str(interface)?.to_idx(),
    };
    Ok(idx)
}

/// The name of the interface at `idx` on `device_model`.
///
/// # Errors
///
/// Fails when the model has no interface at `idx`.
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
        NodeModel::CiscoIol | NodeModel::CiscoIolL2 => CiscoIolInt::from_idx(idx)?,
        NodeModel::CiscoFtdv => CiscoFtdvInt::from_idx(idx)?,
        NodeModel::JuniperVrouter => JuniperVrouterInt::from_idx(idx)?,
        NodeModel::JuniperVswitch => JuniperVswitchInt::from_idx(idx)?,
        NodeModel::JuniperVevolved => JuniperVevolvedInt::from_idx(idx)?,
        NodeModel::JuniperCevolved => JuniperCevolvedInt::from_idx(idx)?,
        NodeModel::JuniperVsrxv3 => JuniperVsrxv3Int::from_idx(idx)?,
        NodeModel::CumulusLinux => CumulusLinuxInt::from_idx(idx)?,
        NodeModel::NokiaSrlinux => NokiaSrlinuxInt::from_idx(idx)?,
        NodeModel::MikrotikChr => MikrotikChrInt::from_idx(idx)?,
        NodeModel::PaloaltoPanos => PaloaltoPanosInt::from_idx(idx)?,
        _ => EthernetInt::from_idx(idx)?,
    };
    Ok(iface)
}

#[must_use]
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
        NodeModel::CiscoIol | NodeModel::CiscoIolL2 => CiscoIolInt::all_interfaces(),
        NodeModel::CiscoFtdv => CiscoFtdvInt::all_interfaces(),
        NodeModel::JuniperVrouter => JuniperVrouterInt::all_interfaces(),
        NodeModel::JuniperVswitch => JuniperVswitchInt::all_interfaces(),
        NodeModel::JuniperVevolved => JuniperVevolvedInt::all_interfaces(),
        NodeModel::JuniperCevolved => JuniperCevolvedInt::all_interfaces(),
        NodeModel::JuniperVsrxv3 => JuniperVsrxv3Int::all_interfaces(),
        NodeModel::CumulusLinux => CumulusLinuxInt::all_interfaces(),
        NodeModel::NokiaSrlinux => NokiaSrlinuxInt::all_interfaces(),
        NodeModel::MikrotikChr => MikrotikChrInt::all_interfaces(),
        NodeModel::PaloaltoPanos => PaloaltoPanosInt::all_interfaces(),
        _ => EthernetInt::all_interfaces(),
    }
}

/// The name of data interface `idx` of a `model` container, inside the container. SR Linux
/// has its own names. cJunosEvolved creates `eth1` to `eth3` itself, so its first data
/// interface is `eth4`. Other models use `eth<idx>`.
///
/// # Errors
///
/// Fails when the model has no interface at `idx`.
pub fn container_interface_name(model: &NodeModel, idx: u8) -> Result<String> {
    match model {
        NodeModel::NokiaSrlinux => srlinux_to_linux_interface(&interface_from_idx(model, idx)?),
        NodeModel::JuniperCevolved => Ok(format!(
            "eth{}",
            u16::from(idx) + CEVOLVED_INTERNAL_INTERFACES
        )),
        _ => Ok(format!("eth{idx}")),
    }
}

/// Convert an SR Linux interface name to the Linux-compatible name used
/// inside the container.  e.g. "eth-1/3" → "e1-3".
///
/// # Errors
///
/// Fails when `name` is not an SR Linux `ethernet-1/N` interface.
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
    use strum::IntoEnumIterator;

    use super::*;
    use crate::data::NodeConfig;

    #[test]
    fn container_interface_names_follow_each_model() {
        assert_eq!(
            container_interface_name(&NodeModel::NokiaSrlinux, 3).unwrap(),
            "e1-3"
        );
        assert_eq!(
            container_interface_name(&NodeModel::JuniperCevolved, 1).unwrap(),
            "eth4"
        );
        assert_eq!(
            container_interface_name(&NodeModel::JuniperCevolved, 36).unwrap(),
            "eth39"
        );
        assert_eq!(
            container_interface_name(&NodeModel::AristaCeos, 2).unwrap(),
            "eth2"
        );
    }

    #[test]
    fn cisco_iol_interfaces_are_ethernet_slot_and_port_and_container_eth_by_index() {
        for model in [NodeModel::CiscoIol, NodeModel::CiscoIolL2] {
            for idx in 0..16u8 {
                let name = format!("Ethernet{}/{}", idx / 4, idx % 4);
                assert_eq!(interface_from_idx(&model, idx).unwrap(), name);
                assert_eq!(interface_to_idx(&model, &name).unwrap(), idx);
                assert_eq!(
                    container_interface_name(&model, idx).unwrap(),
                    format!("eth{idx}")
                );
            }
            assert!(interface_from_idx(&model, 16).is_err());
            assert_eq!(node_model_interfaces(&model).len(), 16);
            assert_eq!(
                NodeConfig::get_model(model)
                    .interfaces
                    .management
                    .to_string(),
                "Ethernet0/0"
            );
        }
    }

    #[test]
    fn cevolved_interfaces_are_the_36_et_ports() {
        let model = NodeModel::JuniperCevolved;
        assert_eq!(interface_from_idx(&model, 1).unwrap(), "et-0/0/0");
        assert_eq!(interface_from_idx(&model, 36).unwrap(), "et-0/0/35");
        assert_eq!(interface_to_idx(&model, "re0:mgmt-0").unwrap(), 0);
        assert!(interface_from_idx(&model, 37).is_err());
    }

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
        assert_ne!(
            node_model_interfaces(&NodeModel::CiscoIosv),
            Vec::<String>::new()
        );
        assert_ne!(
            node_model_interfaces(&NodeModel::NokiaSrlinux),
            Vec::<String>::new()
        );
        assert_ne!(
            node_model_interfaces(&NodeModel::UbuntuLinux),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_first_interface_of_every_model_round_trips() {
        for model in NodeModel::iter() {
            let name = interface_from_idx(&model, 0).unwrap();
            let back = interface_to_idx(&model, &name).unwrap();
            assert_eq!(
                back, 0,
                "{model:?}: round-trip failed for idx 0 (name={name})"
            );
        }
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
            NodeModel::CiscoIol,
            NodeModel::CiscoIolL2,
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
