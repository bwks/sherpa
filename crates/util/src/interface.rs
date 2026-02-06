use std::str::FromStr;

use anyhow::Result;

use data::{
    AristaCeosInt, AristaVeosInt, ArubaAoscxInt, CiscoAsavInt, CiscoCat8000vInt, CiscoCat9000vInt,
    CiscoCsr1000vInt, CiscoFtdvInt, CiscoIosvInt, CiscoIosvl2Int, CiscoIosxrv9000Int,
    CiscoNexus9300vInt, CumulusLinuxInt, EthernetInt, InterfaceKind, InterfaceTrait,
    JuniperVevolvedInt, JuniperVrouterInt, JuniperVsrxv3Int, JuniperVswitchInt, NodeModel,
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
        // DeviceModels::NokiaSrlinux => NokiaSrlinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::WindowsServer2012 => {
        //     WindowsServer2012::from_str(interface)?.to_idx()
        // }
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
        // DeviceModels::NokiaSrlinux => NokiaSrlinuxInt::from_idx(idx)?,
        // DeviceModels::WindowsServer2012 => {
        //     WindowsServer2012::from_idx(idx)?
        // }
        _ => EthernetInt::from_idx(idx)?,
    };
    Ok(iface)
}

pub fn parse_interface_kind(
    device_model: &NodeModel,
    interface_name: &str,
) -> Result<InterfaceKind> {
    let kind = match device_model {
        NodeModel::AristaVeos => {
            InterfaceKind::AristaVeos(AristaVeosInt::from_str(interface_name)?)
        }
        NodeModel::AristaCeos => {
            InterfaceKind::AristaCeos(AristaCeosInt::from_str(interface_name)?)
        }
        NodeModel::ArubaAoscx => {
            InterfaceKind::ArubaAoscx(ArubaAoscxInt::from_str(interface_name)?)
        }
        NodeModel::CiscoAsav => InterfaceKind::CiscoAsav(CiscoAsavInt::from_str(interface_name)?),
        NodeModel::CiscoCsr1000v => {
            InterfaceKind::CiscoCsr1000v(CiscoCsr1000vInt::from_str(interface_name)?)
        }
        NodeModel::CiscoCat8000v => {
            InterfaceKind::CiscoCat8000v(CiscoCat8000vInt::from_str(interface_name)?)
        }
        NodeModel::CiscoCat9000v => {
            InterfaceKind::CiscoCat9000v(CiscoCat9000vInt::from_str(interface_name)?)
        }
        NodeModel::CiscoIosxrv9000 => {
            InterfaceKind::CiscoIosxrv9000(CiscoIosxrv9000Int::from_str(interface_name)?)
        }
        NodeModel::CiscoNexus9300v => {
            InterfaceKind::CiscoNexus9000v(CiscoNexus9300vInt::from_str(interface_name)?)
        }
        NodeModel::CiscoIosv => InterfaceKind::CiscoIosv(CiscoIosvInt::from_str(interface_name)?),
        NodeModel::CiscoIosvl2 => {
            InterfaceKind::CiscoIosvl2(CiscoIosvl2Int::from_str(interface_name)?)
        }
        NodeModel::CiscoFtdv => InterfaceKind::CiscoFtdv(CiscoFtdvInt::from_str(interface_name)?),
        NodeModel::JuniperVrouter => {
            InterfaceKind::JuniperVrouter(JuniperVrouterInt::from_str(interface_name)?)
        }
        NodeModel::JuniperVswitch => {
            InterfaceKind::JuniperVswitch(JuniperVswitchInt::from_str(interface_name)?)
        }
        NodeModel::JuniperVevolved => {
            InterfaceKind::JuniperVevolved(JuniperVevolvedInt::from_str(interface_name)?)
        }
        NodeModel::JuniperVsrxv3 => {
            InterfaceKind::JuniperVsrxv3(JuniperVsrxv3Int::from_str(interface_name)?)
        }
        NodeModel::CumulusLinux => {
            InterfaceKind::CumulusLinux(CumulusLinuxInt::from_str(interface_name)?)
        }
        _ => InterfaceKind::Ethernet(EthernetInt::from_str(interface_name)?),
    };
    Ok(kind)
}
