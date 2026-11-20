use std::str::FromStr;

use anyhow::{Result, bail};

use crate::data::{
    AristaEosContainerInt, AristaEosVmInt, ArubaAoscxInt, CiscoAsaInt, CiscoCat8000Int,
    CiscoCat9000Int, CiscoCsr1000Int, CiscoFtdInt, CiscoIolInt, CiscoIosInt, CiscoIosl2Int,
    CiscoIosxr9000Int, CiscoNexus9300Int, CumulusLinuxInt, EthernetInt, InterfaceTrait,
    JuniperEvolvedContainerInt, JuniperEvolvedVmInt, JuniperRouterInt, JuniperSrxv3Int,
    JuniperSwitchInt, MikrotikChrInt, ModelKind, NodeKind, NodeModel, NokiaSrlinuxInt,
    PaloaltoPanosInt,
};

/// The interfaces cJunosEvolved creates for itself in its container, `eth1` to `eth3`.
const CEVOLVED_INTERNAL_INTERFACES: u16 = 3;

/// The index of the interface named `interface` on a node of `model_kind`.
///
/// # Errors
///
/// Fails when the model has no interface named `interface`.
pub fn interface_to_idx(model_kind: ModelKind, interface: &str) -> Result<u8> {
    let idx = match (model_kind.model(), model_kind.kind()) {
        (NodeModel::AristaEos, NodeKind::VirtualMachine) => {
            AristaEosVmInt::from_str(interface)?.to_idx()
        }
        (NodeModel::AristaEos, NodeKind::Container) => {
            AristaEosContainerInt::from_str(interface)?.to_idx()
        }
        (NodeModel::ArubaAoscx, _) => ArubaAoscxInt::from_str(interface)?.to_idx(),
        (NodeModel::CiscoAsa, _) => CiscoAsaInt::from_str(interface)?.to_idx(),
        (NodeModel::CiscoCsr1000, _) => CiscoCsr1000Int::from_str(interface)?.to_idx(),
        (NodeModel::CiscoCat8000, _) => CiscoCat8000Int::from_str(interface)?.to_idx(),
        (NodeModel::CiscoCat9000, _) => CiscoCat9000Int::from_str(interface)?.to_idx(),
        (NodeModel::CiscoIosxr9000, _) => CiscoIosxr9000Int::from_str(interface)?.to_idx(),
        (NodeModel::CiscoNexus9300, _) => CiscoNexus9300Int::from_str(interface)?.to_idx(),
        (NodeModel::CiscoIos, _) => CiscoIosInt::from_str(interface)?.to_idx(),
        (NodeModel::CiscoIosl2, _) => CiscoIosl2Int::from_str(interface)?.to_idx(),
        (NodeModel::CiscoIol | NodeModel::CiscoIoll2, _) => {
            CiscoIolInt::from_str(interface)?.to_idx()
        }
        (NodeModel::CiscoFtd, _) => CiscoFtdInt::from_str(interface)?.to_idx(),
        (NodeModel::JuniperRouter, _) => JuniperRouterInt::from_str(interface)?.to_idx(),
        (NodeModel::JuniperSwitch, _) => JuniperSwitchInt::from_str(interface)?.to_idx(),
        (NodeModel::JuniperEvolved, NodeKind::VirtualMachine) => {
            JuniperEvolvedVmInt::from_str(interface)?.to_idx()
        }
        (NodeModel::JuniperEvolved, NodeKind::Container) => {
            JuniperEvolvedContainerInt::from_str(interface)?.to_idx()
        }
        (NodeModel::JuniperSrxv3, _) => JuniperSrxv3Int::from_str(interface)?.to_idx(),
        (NodeModel::CumulusLinux, _) => CumulusLinuxInt::from_str(interface)?.to_idx(),
        (NodeModel::NokiaSrlinux, _) => NokiaSrlinuxInt::from_str(interface)?.to_idx(),
        (NodeModel::MikrotikChr, _) => MikrotikChrInt::from_str(interface)?.to_idx(),
        (NodeModel::PaloaltoPanos, _) => PaloaltoPanosInt::from_str(interface)?.to_idx(),
        _ => EthernetInt::from_str(interface)?.to_idx(),
    };
    Ok(idx)
}

/// The name of the interface at `idx` on a node of `model_kind`.
///
/// # Errors
///
/// Fails when the model has no interface at `idx`.
pub fn interface_from_idx(model_kind: ModelKind, idx: u8) -> Result<String> {
    let iface = match (model_kind.model(), model_kind.kind()) {
        (NodeModel::AristaEos, NodeKind::VirtualMachine) => AristaEosVmInt::from_idx(idx)?,
        (NodeModel::AristaEos, NodeKind::Container) => AristaEosContainerInt::from_idx(idx)?,
        (NodeModel::ArubaAoscx, _) => ArubaAoscxInt::from_idx(idx)?,
        (NodeModel::CiscoAsa, _) => CiscoAsaInt::from_idx(idx)?,
        (NodeModel::CiscoCsr1000, _) => CiscoCsr1000Int::from_idx(idx)?,
        (NodeModel::CiscoCat8000, _) => CiscoCat8000Int::from_idx(idx)?,
        (NodeModel::CiscoCat9000, _) => CiscoCat9000Int::from_idx(idx)?,
        (NodeModel::CiscoIosxr9000, _) => CiscoIosxr9000Int::from_idx(idx)?,
        (NodeModel::CiscoNexus9300, _) => CiscoNexus9300Int::from_idx(idx)?,
        (NodeModel::CiscoIos, _) => CiscoIosInt::from_idx(idx)?,
        (NodeModel::CiscoIosl2, _) => CiscoIosl2Int::from_idx(idx)?,
        (NodeModel::CiscoIol | NodeModel::CiscoIoll2, _) => CiscoIolInt::from_idx(idx)?,
        (NodeModel::CiscoFtd, _) => CiscoFtdInt::from_idx(idx)?,
        (NodeModel::JuniperRouter, _) => JuniperRouterInt::from_idx(idx)?,
        (NodeModel::JuniperSwitch, _) => JuniperSwitchInt::from_idx(idx)?,
        (NodeModel::JuniperEvolved, NodeKind::VirtualMachine) => {
            JuniperEvolvedVmInt::from_idx(idx)?
        }
        (NodeModel::JuniperEvolved, NodeKind::Container) => {
            JuniperEvolvedContainerInt::from_idx(idx)?
        }
        (NodeModel::JuniperSrxv3, _) => JuniperSrxv3Int::from_idx(idx)?,
        (NodeModel::CumulusLinux, _) => CumulusLinuxInt::from_idx(idx)?,
        (NodeModel::NokiaSrlinux, _) => NokiaSrlinuxInt::from_idx(idx)?,
        (NodeModel::MikrotikChr, _) => MikrotikChrInt::from_idx(idx)?,
        (NodeModel::PaloaltoPanos, _) => PaloaltoPanosInt::from_idx(idx)?,
        _ => EthernetInt::from_idx(idx)?,
    };
    Ok(iface)
}

/// Every interface name a node of `model_kind` has, in index order.
#[must_use]
pub fn node_model_interfaces(model_kind: ModelKind) -> Vec<String> {
    match (model_kind.model(), model_kind.kind()) {
        (NodeModel::AristaEos, NodeKind::VirtualMachine) => AristaEosVmInt::all_interfaces(),
        (NodeModel::AristaEos, NodeKind::Container) => AristaEosContainerInt::all_interfaces(),
        (NodeModel::ArubaAoscx, _) => ArubaAoscxInt::all_interfaces(),
        (NodeModel::CiscoAsa, _) => CiscoAsaInt::all_interfaces(),
        (NodeModel::CiscoCsr1000, _) => CiscoCsr1000Int::all_interfaces(),
        (NodeModel::CiscoCat8000, _) => CiscoCat8000Int::all_interfaces(),
        (NodeModel::CiscoCat9000, _) => CiscoCat9000Int::all_interfaces(),
        (NodeModel::CiscoIosxr9000, _) => CiscoIosxr9000Int::all_interfaces(),
        (NodeModel::CiscoNexus9300, _) => CiscoNexus9300Int::all_interfaces(),
        (NodeModel::CiscoIos, _) => CiscoIosInt::all_interfaces(),
        (NodeModel::CiscoIosl2, _) => CiscoIosl2Int::all_interfaces(),
        (NodeModel::CiscoIol | NodeModel::CiscoIoll2, _) => CiscoIolInt::all_interfaces(),
        (NodeModel::CiscoFtd, _) => CiscoFtdInt::all_interfaces(),
        (NodeModel::JuniperRouter, _) => JuniperRouterInt::all_interfaces(),
        (NodeModel::JuniperSwitch, _) => JuniperSwitchInt::all_interfaces(),
        (NodeModel::JuniperEvolved, NodeKind::VirtualMachine) => {
            JuniperEvolvedVmInt::all_interfaces()
        }
        (NodeModel::JuniperEvolved, NodeKind::Container) => {
            JuniperEvolvedContainerInt::all_interfaces()
        }
        (NodeModel::JuniperSrxv3, _) => JuniperSrxv3Int::all_interfaces(),
        (NodeModel::CumulusLinux, _) => CumulusLinuxInt::all_interfaces(),
        (NodeModel::NokiaSrlinux, _) => NokiaSrlinuxInt::all_interfaces(),
        (NodeModel::MikrotikChr, _) => MikrotikChrInt::all_interfaces(),
        (NodeModel::PaloaltoPanos, _) => PaloaltoPanosInt::all_interfaces(),
        _ => EthernetInt::all_interfaces(),
    }
}

/// The name of data interface `idx` of a `model_kind` container, inside the container. SR
/// Linux has its own names. cJunosEvolved creates `eth1` to `eth3` itself, so its first data
/// interface is `eth4`. Other models use `eth<idx>`.
///
/// # Errors
///
/// Fails when the model has no interface at `idx`.
pub fn container_interface_name(model_kind: ModelKind, idx: u8) -> Result<String> {
    match model_kind.model() {
        NodeModel::NokiaSrlinux => {
            srlinux_to_linux_interface(&interface_from_idx(model_kind, idx)?)
        }
        NodeModel::JuniperEvolved if model_kind.kind() == NodeKind::Container => Ok(format!(
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
    use super::*;

    #[test]
    fn node_interfaces_002_a_containers_data_interfaces_are_named_as_its_model_needs() {
        assert_eq!(
            container_interface_name(ModelKind::default_for(NodeModel::NokiaSrlinux), 3).unwrap(),
            "e1-3"
        );
        assert_eq!(
            container_interface_name(
                ModelKind::new(NodeModel::JuniperEvolved, Some(NodeKind::Container)).unwrap(),
                1
            )
            .unwrap(),
            "eth4"
        );
        assert_eq!(
            container_interface_name(
                ModelKind::new(NodeModel::JuniperEvolved, Some(NodeKind::Container)).unwrap(),
                36
            )
            .unwrap(),
            "eth39"
        );
        assert_eq!(
            container_interface_name(
                ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container)).unwrap(),
                2
            )
            .unwrap(),
            "eth2"
        );

        assert_eq!(srlinux_to_linux_interface("eth-1/1").unwrap(), "e1-1");
        assert_eq!(srlinux_to_linux_interface("eth-1/3").unwrap(), "e1-3");
        assert_eq!(srlinux_to_linux_interface("eth-1/34").unwrap(), "e1-34");

        assert!(srlinux_to_linux_interface("mgmt0").is_err());

        for model in [NodeModel::CiscoIol, NodeModel::CiscoIoll2] {
            for idx in 0..16u8 {
                assert_eq!(
                    container_interface_name(ModelKind::default_for(model), idx).unwrap(),
                    format!("eth{idx}")
                );
            }
        }
    }

    #[test]
    fn node_interfaces_001_a_named_interface_is_that_interface_for_each_model_and_kind() {
        for model_kind in ModelKind::all() {
            let name = interface_from_idx(model_kind, 0).unwrap();
            assert_eq!(
                interface_to_idx(model_kind, &name).unwrap(),
                0,
                "{model_kind:?}: {name}"
            );
        }

        let veos = ModelKind::default_for(NodeModel::AristaEos);
        let ceos = ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container)).unwrap();
        assert_eq!(interface_from_idx(veos, 0).unwrap(), "Management1");
        assert_eq!(interface_from_idx(ceos, 0).unwrap(), "Management0");
        assert_eq!(container_interface_name(ceos, 2).unwrap(), "eth2");

        let vevolved = ModelKind::default_for(NodeModel::JuniperEvolved);
        let cevolved =
            ModelKind::new(NodeModel::JuniperEvolved, Some(NodeKind::Container)).unwrap();
        assert_eq!(interface_from_idx(cevolved, 36).unwrap(), "et-0/0/35");
        assert!(interface_from_idx(vevolved, 36).is_err());
        assert_eq!(container_interface_name(cevolved, 1).unwrap(), "eth4");

        let vm = ModelKind::default_for(NodeModel::SonicLinux);
        let container = ModelKind::new(NodeModel::SonicLinux, Some(NodeKind::Container)).unwrap();
        for model_kind in [vm, container] {
            assert_eq!(interface_from_idx(model_kind, 0).unwrap(), "eth0");
            assert_eq!(interface_from_idx(model_kind, 1).unwrap(), "eth1");
            assert_eq!(interface_from_idx(model_kind, 32).unwrap(), "eth32");
            assert_eq!(interface_to_idx(model_kind, "eth32").unwrap(), 32);
        }
        for idx in 1..=32u8 {
            assert_eq!(
                container_interface_name(container, idx).unwrap(),
                format!("eth{idx}")
            );
        }

        let cevolved =
            ModelKind::new(NodeModel::JuniperEvolved, Some(NodeKind::Container)).unwrap();
        assert_eq!(interface_from_idx(cevolved, 1).unwrap(), "et-0/0/0");
        assert_eq!(interface_from_idx(cevolved, 36).unwrap(), "et-0/0/35");
        assert_eq!(interface_to_idx(cevolved, "re0:mgmt-0").unwrap(), 0);
        assert!(interface_from_idx(cevolved, 37).is_err());

        for model in [NodeModel::CiscoIol, NodeModel::CiscoIoll2] {
            for idx in 0..16u8 {
                let name = format!("Ethernet{}/{}", idx / 4, idx % 4);
                assert_eq!(
                    interface_from_idx(ModelKind::default_for(model), idx).unwrap(),
                    name
                );
                assert_eq!(
                    interface_to_idx(ModelKind::default_for(model), &name).unwrap(),
                    idx
                );
            }
            assert!(interface_from_idx(ModelKind::default_for(model), 16).is_err());
            assert_eq!(
                node_model_interfaces(ModelKind::default_for(model)).len(),
                16
            );
            assert_eq!(
                ModelKind::default_for(model)
                    .node_config()
                    .interfaces
                    .management
                    .to_string(),
                "Ethernet0/0"
            );
        }
    }
}
