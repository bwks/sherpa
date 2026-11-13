mod auth;
mod backup;
mod cluster;
mod config;
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
mod lab;
mod lab_lifecycle;
mod link_report;
mod mapping;
mod network;
mod node;
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
pub use cluster::{ClusterCni, ClusterRole};

pub use config::{
    ClientConfig, Config, HostMemorySettings, OtelConfig, ScannerConfig, ServerConnection, Sherpa,
    TailscaleGatewaySettings, TlsConfig, ZtpServer,
};
pub use container::{ContainerImage, ContainerNetworkAttachment};
pub use cpu::{CpuFeature, CpuFeaturePolicy, CpuModels};
pub use db::{
    BridgeMember, DbBridge, DbLab, DbLink, DbNode, DbSession, DbSessionUser, DbUser, NodeResources,
    SessionChannel,
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
    AristaCeosInt, AristaVeosInt, ArubaAoscxInt, CiscoAsavInt, CiscoCat8000vInt, CiscoCat9000vInt,
    CiscoCsr1000vInt, CiscoFtdvInt, CiscoIosvInt, CiscoIosvl2Int, CiscoIosxrv9000Int,
    CiscoNexus9300vInt, ConnectionTypes, CumulusLinuxInt, EthernetInt, Interface, InterfaceTrait,
    JuniperCevolvedInt, JuniperVevolvedInt, JuniperVrouterInt, JuniperVsrxv3Int, JuniperVswitchInt,
    MgmtInterfaces, MikrotikChrInt, NokiaSrlinuxInt, PaloaltoPanosInt,
};
pub use lab::{
    BridgeInterface, InterfaceData, InterfaceState, LabIdentity, LabInfo, LabIsolatedNetwork,
    LabLinkData, LabNodeData, LabReservedNetwork, LabSshKey, LabState, LabSummary,
    ListLabsResponse, NodeInterface, NodeSetupData, NodeSshUsername, PeerInterface, PeerSide,
};
pub use lab_lifecycle::{LabEvent, LabNotReady, LabPhase, LabRefused, LabTransition};
pub use link_report::{LinkReport, LinkState};
pub use mapping::{CloneDisk, NodeDisk, QemuCommand};
pub use network::{BridgeKind, LabSubnets, NetworkV4, NetworkV6, SherpaNetwork};
pub use node::{
    BiosTypes, BootDiskSettings, CdromSettings, CpuArchitecture, CpuSettings, InterfaceSettings,
    InterfaceType, MachineSettings, MachineType, NodeConfig, NodeKind, NodeModel, NodeState,
    OsVariant, UnikernelBootMode, UnikernelSettings, Ztp, ZtpSettings,
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
    ClusterInfo, KeycloakInfo, KeycloakUser, NodeInfo, StatusKind, StatusMessage, UpError, UpPhase,
    UpRequest, UpResponse, UpSummary, VaultInfo,
};
pub use user::{OwnerSshKey, OwnerUser, User};
pub use user_management::{
    AddSshKeyRequest, ChangePasswordRequest, ChangePasswordResponse, CreateUserRequest,
    CreateUserResponse, DeleteUserRequest, DeleteUserResponse, GetUserInfoRequest,
    GetUserInfoResponse, ListUsersRequest, ListUsersResponse, RemoveSshKeyRequest, UserInfo,
    UserSshKeysResponse,
};
pub use vm_action::{CancelLabResponse, LabNodeActionResponse, NodeActionResult};
pub use watchdog::WatchdogAction;
pub use ws::ConnectedMsg;
pub use ztp::ZtpRecord;

pub mod tailscale;
pub use tailscale::{RouteApproval, TailnetAuthKey, TailnetState, TailnetStatus};
