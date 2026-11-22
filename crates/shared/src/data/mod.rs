mod auth;
mod backup;
mod cluster;
mod config;
mod console;
mod container;
mod cpu;
mod db;
mod destroy;
mod disk;
mod dns;
mod download;
mod host_key;
mod impairment;
mod import;
mod inspect;
mod interface;
pub mod kind_settings;
mod lab;
mod lab_lifecycle;
mod link_report;
mod mapping;
mod model_kind;
mod network;
mod node;
mod node_base;
mod node_lifecycle;
mod performance;
mod provider;
mod provision;
mod ptx_platform;
mod record_id;
mod redeploy;
mod secret;
mod ssh;
mod up;
mod user;
mod user_management;
mod vm_action;
mod watchdog;
mod ws;
mod ztp;

pub use auth::{LoginRequest, LoginResponse, LogoutResponse, ValidateRequest, ValidateResponse};
pub use backup::BackupResponse;
pub use cluster::{AddressRange, ClusterCni, ClusterMember, ClusterPlan, ClusterRole};

pub use config::{
    ClientConfig, Config, HostMemorySettings, OtelConfig, ScannerConfig, ServerConnection, Sherpa,
    TailscaleGatewaySettings, TlsConfig,
};
pub use console::ConsoleCheck;
pub use container::{ContainerImage, ContainerNetworkAttachment};
pub use cpu::{CpuFeature, CpuFeaturePolicy, CpuModels};
pub use db::{
    BridgeMember, DbBridge, DbCpuPins, DbLab, DbLink, DbNode, DbSession, DbSessionUser, DbUser,
    NodeResources, ReadyCheck, SessionChannel,
};
pub use destroy::{DestroyError, DestroyRequest, DestroyResponse, DestroySummary};
pub use disk::{
    DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskImageCloneMethod, DiskTargets,
};
pub use dns::{Dns, NameServer};
pub use download::DownloadLabResponse;
pub use host_key::{NodeHostKey, NodeHostKeysRequest, NodeHostKeysResponse};
pub use impairment::{
    ImpairmentDirection, LinkImpairment, UpdateImpairmentRequest, UpdateImpairmentResponse,
};
pub use import::{
    ContainerPullRequest, ContainerPullResponse, DeleteImageRequest, DeleteImageResponse,
    DownloadImageRequest, ImageSummary, ImportRequest, ImportResponse, ListImagesRequest,
    ListImagesResponse, ScanImagesRequest, ScanImagesResponse, ScannedImage,
    SetDefaultImageRequest, SetDefaultImageResponse, ShowImageRequest, ShowImageResponse,
    SkippedModel,
};
pub use inspect::{BridgeInfo, DeviceInfo, InspectRequest, InspectResponse, LinkInfo};
pub use interface::{
    AristaEosContainerInt, AristaEosVmInt, ArubaAoscxInt, CiscoAsaInt, CiscoCat8000Int,
    CiscoCat9000Int, CiscoCsr1000Int, CiscoFtdInt, CiscoIolInt, CiscoIosInt, CiscoIosl2Int,
    CiscoIosxr9000Int, CiscoNexus9300Int, ConnectionTypes, CumulusLinuxInt, EthernetInt, Interface,
    InterfaceTrait, JuniperEvolvedContainerInt, JuniperEvolvedVmInt, JuniperRouterInt,
    JuniperSrxv3Int, JuniperSwitchInt, MgmtInterfaces, MikrotikChrInt, NokiaSrlinuxInt,
    PaloaltoPanosInt,
};
pub use lab::{
    BridgeInterface, InterfaceData, InterfaceState, LabIdentity, LabInfo, LabIsolatedNetwork,
    LabLinkData, LabNodeData, LabReservedNetwork, LabState, LabSummary, ListLabsResponse,
    NodeInterface, NodeSetupData, NodeSshUsername, PeerInterface, PeerSide,
};
pub use lab_lifecycle::{LabEvent, LabNotReady, LabPhase, LabRefused, LabTransition};
pub use link_report::{LinkReport, LinkState};
pub use mapping::{CloneDisk, NodeDisk, QemuCommand};
pub use model_kind::{ModelKind, model_kinds_list};
pub use network::{BridgeKind, LabSubnets, NetworkV4, NetworkV6, SherpaNetwork};
pub use node::{
    BiosTypes, BootDiskSettings, CdromSettings, ContainerSettings, CpuArchitecture, CpuSettings,
    InterfaceDeviceSettings, InterfaceSettings, InterfaceType, KindSettings, MachineSettings,
    MachineType, NodeConfig, NodeKind, NodeModel, NodeState, OsVariant, REMOVED_MODELS,
    UnikernelBootMode, UnikernelCpuSettings, UnikernelMachineSettings, UnikernelSettings,
    VmCpuSettings, VmSettings, Ztp, ZtpSettings,
};
pub use node_lifecycle::{Halt, Observation, OperationEvent, Probe, Refused, Runtime};
pub use performance::{
    CpuMode, CpuTopology, DiskCache, DiskIo, NodeBootDisk, NodeCpu, NodeCpuPins, NodeInterfaces,
    NodeNuma, NumaMemoryMode, cpuset,
};
pub use provider::VmProviders;
pub use provision::{ProvisionResult, ProvisionStatus};
pub use ptx_platform::PtxPlatform;
pub use record_id::{RecordId, RecordIdKey};
pub use redeploy::{RedeployRequest, RedeployResponse};
pub use secret::{NodeSecretsRequest, NodeSecretsResponse, SecretContent, SecretFile};
pub use ssh::{SshKeyAlgorithms, SshPublicKey};
pub use up::{
    AdminCredential, ClusterInfo, KeycloakInfo, KeycloakUser, NodeInfo, StatusKind, StatusMessage,
    SurrealDbInfo, UpError, UpPhase, UpRequest, UpResponse, UpSummary, VaultInfo,
};
pub use user::{OwnerSshKey, OwnerUser};
pub use user_management::{
    AddSshKeyRequest, ChangePasswordRequest, ChangePasswordResponse, CreateUserRequest,
    CreateUserResponse, DeleteUserRequest, DeleteUserResponse, GetUserInfoRequest,
    GetUserInfoResponse, ListUsersRequest, ListUsersResponse, RemoveSshKeyRequest,
    SetDevicePasswordRequest, SetDevicePasswordResponse, UserInfo, UserSshKeysResponse,
};
pub use vm_action::{CancelLabResponse, LabNodeActionResponse, NodeActionResult};
pub use watchdog::WatchdogAction;
pub use ws::ConnectedMsg;
pub use ztp::ZtpRecord;

pub mod tailscale;
pub use tailscale::{RouteApproval, TailnetAuthKey, TailnetState, TailnetStatus};
