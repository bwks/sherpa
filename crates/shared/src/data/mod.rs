mod config;
mod container;
mod cpu;
mod db;
mod dhcp;
mod disk;
mod dns;
mod interface;
mod lab;
mod mapping;
mod network;
mod node;
mod provider;
mod ssh;
mod user;
mod ztp;

pub use config::{Config, ConfigurationManagement, Sherpa, ZtpServer};
pub use container::{ContainerImage, ContainerModel, ContainerNetworkAttachment};
pub use cpu::CpuModels;
pub use db::{DbBridge, DbLab, DbLink, DbNode, DbUser};
pub use dhcp::DhcpLease;
pub use disk::{DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskTargets};
pub use dns::{Dns, NameServer};
pub use interface::{
    AristaCeosInt, AristaVeosInt, ArubaAoscxInt, CiscoAsavInt, CiscoCat8000vInt, CiscoCat9000vInt,
    CiscoCsr1000vInt, CiscoFtdvInt, CiscoIosvInt, CiscoIosvl2Int, CiscoIosxrv9000Int,
    CiscoNexus9300vInt, ConnectionTypes, CumulusLinuxInt, EthernetInt, Interface, InterfaceTrait,
    JuniperVevolvedInt, JuniperVrouterInt, JuniperVsrxv3Int, JuniperVswitchInt, MgmtInterfaces,
};
pub use lab::{
    BridgeConnection, BridgeInterface, InterfaceData, InterfaceState, LabBridgeData, LabInfo,
    LabIsolatedNetwork, LabLinkData, LabNodeData, LabReservedNetwork, NodeInterface, NodeSetupData,
    PeerInterface, PeerSide,
};
pub use mapping::{CloneDisk, InterfaceConnection, NodeConnection, NodeDisk, QemuCommand};
pub use network::{BridgeKind, NetworkV4, SherpaNetwork};
pub use node::{
    BiosTypes, CpuArchitecture, InterfaceType, MachineType, NodeConfig, NodeKind, NodeModel,
    OsVariant, ZtpMethod,
};
pub use provider::VmProviders;
pub use ssh::{SshKeyAlgorithms, SshPublicKey};
pub use user::User;
pub use ztp::ZtpRecord;

// Re-export SurrealDB types for convenience
pub use surrealdb::RecordId;
