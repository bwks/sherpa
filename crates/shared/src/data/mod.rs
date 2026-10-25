mod auth;
mod cluster;
mod config;
mod container;
mod cpu;
mod db;
mod destroy;
mod dhcp;
mod disk;
mod dns;
mod download;
mod impairment;
mod import;
mod inspect;
mod interface;
mod lab;
mod mapping;
mod network;
mod node;
mod provider;
mod provision;
mod record_id;
mod redeploy;
mod secret;
mod ssh;
mod up;
mod user;
mod user_management;
mod vm_action;
mod ws;
mod ztp;

pub use auth::{LoginRequest, LoginResponse, ValidateRequest, ValidateResponse};
pub use cluster::ClusterRole;

pub use config::{
    ClientConfig, Config, OtelConfig, ScannerConfig, ServerConnection, Sherpa,
    TailscaleGatewaySettings, TlsConfig, ZtpServer,
};
pub use container::{ContainerImage, ContainerModel, ContainerNetworkAttachment};
pub use cpu::{CpuFeature, CpuFeaturePolicy, CpuModels};
pub use db::{DbBridge, DbLab, DbLink, DbNode, DbUser, NodeResources};
pub use destroy::{DestroyError, DestroyRequest, DestroyResponse, DestroySummary};
pub use dhcp::DhcpLease;
pub use disk::{
    DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskImageCloneMethod, DiskTargets,
};
pub use dns::{Dns, NameServer};
pub use download::DownloadLabResponse;
pub use impairment::{UpdateImpairmentRequest, UpdateImpairmentResponse};
pub use import::{
    ContainerPullRequest, ContainerPullResponse, DeleteImageRequest, DeleteImageResponse,
    DownloadImageRequest, ImageSummary, ImportRequest, ImportResponse, ListImagesRequest,
    ListImagesResponse, ScanImagesRequest, ScanImagesResponse, ScannedImage,
    SetDefaultImageRequest, SetDefaultImageResponse, ShowImageRequest, ShowImageResponse,
};
pub use inspect::{BridgeInfo, DeviceInfo, InspectRequest, InspectResponse, LinkInfo};
pub use interface::{
    AristaCeosInt, AristaVeosInt, ArubaAoscxInt, CiscoAsavInt, CiscoCat8000vInt, CiscoCat9000vInt,
    CiscoCsr1000vInt, CiscoFtdvInt, CiscoIosvInt, CiscoIosvl2Int, CiscoIosxrv9000Int,
    CiscoNexus9300vInt, ConnectionTypes, CumulusLinuxInt, EthernetInt, Interface, InterfaceTrait,
    JuniperVevolvedInt, JuniperVrouterInt, JuniperVsrxv3Int, JuniperVswitchInt, MgmtInterfaces,
    MikrotikChrInt, NokiaSrlinuxInt, PaloaltoPanosInt,
};
pub use lab::{
    BridgeConnection, BridgeInterface, InterfaceData, InterfaceState, LabBridgeData, LabIdentity,
    LabInfo, LabIsolatedNetwork, LabLinkData, LabNodeData, LabReservedNetwork, LabState,
    LabSummary, ListLabsResponse, NodeInterface, NodeSetupData, NodeSshUsername, PeerInterface,
    PeerSide,
};
pub use mapping::{CloneDisk, InterfaceConnection, NodeConnection, NodeDisk, QemuCommand};
pub use network::{BridgeKind, LabSubnets, NetworkV4, NetworkV6, SherpaNetwork};
pub use node::{
    BiosTypes, CpuArchitecture, InterfaceType, MachineType, NodeConfig, NodeKind, NodeModel,
    NodeState, OsVariant, UnikernelBootMode, ZtpMethod,
};
pub use provider::VmProviders;
pub use provision::{ProvisionResult, ProvisionStatus};
pub use record_id::{RecordId, RecordIdKey};
pub use redeploy::{RedeployRequest, RedeployResponse};
pub use secret::{NodeSecretsRequest, NodeSecretsResponse, SecretContent, SecretFile};
pub use ssh::{SshKeyAlgorithms, SshPublicKey};
pub use up::{
    ClusterInfo, NodeInfo, StatusKind, StatusMessage, UpError, UpPhase, UpRequest, UpResponse,
    UpSummary,
};
pub use user::User;
pub use user_management::{
    AddSshKeyRequest, ChangePasswordRequest, ChangePasswordResponse, CreateUserRequest,
    CreateUserResponse, DeleteUserRequest, DeleteUserResponse, GetUserInfoRequest,
    GetUserInfoResponse, ListUsersRequest, ListUsersResponse, RemoveSshKeyRequest, UserInfo,
    UserSshKeysResponse,
};
pub use vm_action::{CancelLabResponse, LabNodeActionResponse, NodeActionResult};
pub use ws::ConnectedMsg;
pub use ztp::ZtpRecord;

pub mod tailscale;
pub use tailscale::{RouteApproval, TAILNET_DOCS_URL, TailnetAuthKey, TailnetState, TailnetStatus};
