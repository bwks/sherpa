mod device;
mod interface;
mod provider;

pub use crate::model::device::{
    CpuArchitecture, DeviceModel, DeviceModels, InterfaceTypes, MachineTypes, Manufacturers,
    OsVariants,
};
pub use crate::model::provider::VmProviders;
