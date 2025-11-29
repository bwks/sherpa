use std::str::FromStr;

use anyhow::Result;

use data::{
    AristaCeosInt, AristaVeosInt, ArubaAoscxInt, CiscoAsavInt, CiscoCat8000vInt, CiscoCat9000vInt,
    CiscoCsr1000vInt, CiscoIosvInt, CiscoIosvl2Int, CiscoNexus9300vInt, DeviceModels, EthernetInt,
    InterfaceTrait, JuniperVevolvedInt, JuniperVrouterInt, JuniperVsrxv3Int, JuniperVswitchInt,
};

pub fn interface_to_idx(device_model: DeviceModels, interface: &str) -> Result<u8> {
    let idx = match device_model {
        DeviceModels::CustomUnknown => EthernetInt::from_str(interface)?.to_idx(),
        DeviceModels::AristaVeos => AristaVeosInt::from_str(interface)?.to_idx(),
        DeviceModels::AristaCeos => AristaCeosInt::from_str(interface)?.to_idx(),
        DeviceModels::ArubaAoscx => ArubaAoscxInt::from_str(interface)?.to_idx(),
        DeviceModels::CiscoAsav => CiscoAsavInt::from_str(interface)?.to_idx(),
        DeviceModels::CiscoCsr1000v => CiscoCsr1000vInt::from_str(interface)?.to_idx(),
        DeviceModels::CiscoCat8000v => CiscoCat8000vInt::from_str(interface)?.to_idx(),
        DeviceModels::CiscoCat9000v => CiscoCat9000vInt::from_str(interface)?.to_idx(),
        // DeviceModels::CiscoIosxrv9000 => {
        //     CiscoIosxrv9000Int::from_str(interface)?.to_idx()
        // }
        DeviceModels::CiscoNexus9300v => CiscoNexus9300vInt::from_str(interface)?.to_idx(),
        DeviceModels::CiscoIosv => CiscoIosvInt::from_str(interface)?.to_idx(),
        DeviceModels::CiscoIosvl2 => CiscoIosvl2Int::from_str(interface)?.to_idx(),
        DeviceModels::JuniperVrouter => JuniperVrouterInt::from_str(interface)?.to_idx(),
        DeviceModels::JuniperVswitch => JuniperVswitchInt::from_str(interface)?.to_idx(),
        DeviceModels::JuniperVevolved => JuniperVevolvedInt::from_str(interface)?.to_idx(),
        DeviceModels::JuniperVsrxv3 => JuniperVsrxv3Int::from_str(interface)?.to_idx(),
        // DeviceModels::NokiaSrlinux => NokiaSrlinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::AlpineLinux => AlpineLinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::CumulusLinux => CumulusLinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::CentosLinux => CentosLinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::FedoraLinux => FedoraLinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::RedhatLinux => RedhatLinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::OpensuseLinux => {
        //     OpensuseLinuxInt::from_str(interface)?.to_idx()
        // }
        // DeviceModels::SuseLinux => SuseLinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::UbuntuLinux => UbuntuLinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::FlatcarLinux => FlatcarLinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::SonicLinux => SonicLinuxInt::from_str(interface)?.to_idx(),
        // DeviceModels::WindowsServer2012 => {
        //     WindowsServer2012::from_str(interface)?.to_idx()
        // }
        _ => {
            // println!("ADD MORE MODELS")
            0
        }
    };
    Ok(idx)
}
