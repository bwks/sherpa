mod device;
mod interface;
mod provider;

pub use crate::model::device::{
    BiosTypes, CpuArchitecture, DeviceModel, DeviceModels, InterfaceTypes, MachineTypes,
};
pub use crate::model::interface::{ConnectionTypes, Interface};
pub use crate::model::provider::VmProviders;
