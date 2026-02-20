// Macro to implement SurrealValue for enums that use serde serialization
// This works around the SurrealValue derive macro not respecting serde rename attributes
macro_rules! impl_surreal_value_for_enum {
    ($enum_type:ty) => {
        impl surrealdb_types::SurrealValue for $enum_type {
            fn kind_of() -> surrealdb_types::Kind {
                surrealdb_types::kind!(string)
            }

            fn is_value(value: &surrealdb_types::Value) -> bool {
                matches!(value, surrealdb_types::Value::String(_))
            }

            fn into_value(self) -> surrealdb_types::Value {
                // Use serde to serialize to JSON, then convert to SurrealDB String Value
                // This respects all serde rename attributes
                let json_value = serde_json::to_value(self).expect(concat!(
                    stringify!($enum_type),
                    " serialization should never fail (BALLSACK)"
                ));
                // Convert the serde_json::Value to surrealdb_types::Value
                surrealdb_types::SurrealValue::into_value(json_value)
            }

            fn from_value(value: surrealdb_types::Value) -> Result<Self, surrealdb_types::Error> {
                // Convert SurrealDB Value to serde_json::Value, then use serde to deserialize
                let json_value = surrealdb_types::SurrealValue::from_value(value)?;
                serde_json::from_value(json_value).map_err(|e| {
                    surrealdb_types::Error::internal(format!(
                        concat!("Failed to deserialize ", stringify!($enum_type), ": {}"),
                        e
                    ))
                })
            }
        }
    };
}

mod auth;
mod config;
mod container;
mod cpu;
mod db;
mod destroy;
mod dhcp;
mod disk;
mod dns;
mod import;
mod inspect;
mod interface;
mod lab;
mod mapping;
mod network;
mod node;
mod provider;
mod ssh;
mod up;
mod user;
mod user_management;
mod ztp;

pub use auth::{LoginRequest, LoginResponse, ValidateRequest, ValidateResponse};

pub use config::{Config, ConfigurationManagement, ServerConnection, Sherpa, TlsConfig, ZtpServer};
pub use container::{ContainerImage, ContainerModel, ContainerNetworkAttachment};
pub use cpu::CpuModels;
pub use db::{DbBridge, DbLab, DbLink, DbNode, DbUser};
pub use destroy::{DestroyError, DestroyRequest, DestroyResponse, DestroySummary};
pub use dhcp::DhcpLease;
pub use disk::{DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskTargets};
pub use dns::{Dns, NameServer};
pub use import::{ImportRequest, ImportResponse};
pub use inspect::{BridgeInfo, DeviceInfo, InspectRequest, InspectResponse, LinkInfo};
pub use interface::{
    AristaCeosInt, AristaVeosInt, ArubaAoscxInt, CiscoAsavInt, CiscoCat8000vInt, CiscoCat9000vInt,
    CiscoCsr1000vInt, CiscoFtdvInt, CiscoIosvInt, CiscoIosvl2Int, CiscoIosxrv9000Int,
    CiscoNexus9300vInt, ConnectionTypes, CumulusLinuxInt, EthernetInt, Interface, InterfaceTrait,
    JuniperVevolvedInt, JuniperVrouterInt, JuniperVsrxv3Int, JuniperVswitchInt, MgmtInterfaces,
};
pub use lab::{
    BridgeConnection, BridgeInterface, InterfaceData, InterfaceState, LabBridgeData, LabInfo,
    LabIsolatedNetwork, LabLinkData, LabNodeData, LabReservedNetwork, LabStatus, LabSummary,
    ListLabsResponse, NodeInterface, NodeSetupData, PeerInterface, PeerSide,
};
pub use mapping::{CloneDisk, InterfaceConnection, NodeConnection, NodeDisk, QemuCommand};
pub use network::{BridgeKind, NetworkV4, SherpaNetwork};
pub use node::{
    BiosTypes, CpuArchitecture, InterfaceType, MachineType, NodeConfig, NodeKind, NodeModel,
    NodeState, OsVariant, ZtpMethod,
};
pub use provider::VmProviders;
pub use ssh::{SshKeyAlgorithms, SshPublicKey};
pub use up::{NodeInfo, UpError, UpPhase, UpRequest, UpResponse, UpSummary};
pub use user::User;
pub use user_management::{
    ChangePasswordRequest, ChangePasswordResponse, CreateUserRequest, CreateUserResponse,
    DeleteUserRequest, DeleteUserResponse, GetUserInfoRequest, GetUserInfoResponse,
    ListUsersRequest, ListUsersResponse, UserInfo,
};
pub use ztp::ZtpRecord;

// Re-export SurrealDB types for convenience
pub use surrealdb_types::RecordId;
