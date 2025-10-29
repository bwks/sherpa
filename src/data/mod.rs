mod container;
mod cpu;
mod device;
mod disk;
mod dns;
mod interface;
mod mapping;
mod provider;
mod ssh;
mod user;

pub use crate::data::container::{ContainerImage, ContainerModel};
pub use crate::data::cpu::CpuModels;
pub use crate::data::device::{
    BiosTypes, CpuArchitecture, DeviceKind, DeviceModel, DeviceModels, InterfaceTypes,
    MachineTypes, OsVariants, ZtpMethods,
};
pub use crate::data::disk::{DiskBuses, DiskDevices, DiskDrivers, DiskFormats, DiskTargets};
pub use crate::data::dns::Dns;
pub use crate::data::interface::{ConnectionTypes, Interface, MgmtInterfaces};
pub use crate::data::mapping::{
    BootServer, CloneDisk, DeviceConnection, DeviceDisk, InterfaceConnection, QemuCommand,
    ZtpTemplates,
};
pub use crate::data::provider::VmProviders;
pub use crate::data::ssh::{SshKeyAlgorithms, SshPublicKey};
pub use crate::data::user::User;
