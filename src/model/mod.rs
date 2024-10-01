mod device;
mod interface;
mod provider;
mod ssh;
mod user;

pub use crate::model::device::{
    BiosTypes, CpuArchitecture, DeviceModel, DeviceModels, InterfaceTypes, MachineTypes,
    OsVariants, ZtpMethods,
};
pub use crate::model::interface::{ConnectionTypes, Interface};
pub use crate::model::provider::VmProviders;
pub use crate::model::ssh::{SshKeyAlgorithms, SshPublicKey};
pub use crate::model::user::User;
