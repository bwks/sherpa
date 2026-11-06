//! Node image table schema
//!
//! One row per imported image version of a node model. `(model, kind, version)` is unique.
//! A node image used by a node cannot be deleted (`node.image` is `ON DELETE RESTRICT`).
use anyhow::Result;
use shared::data::{
    BiosTypes, CpuArchitecture, CpuModels, DiskBuses, InterfaceType, MachineType, MgmtInterfaces,
    NodeKind, NodeModel, OsVariant, UnikernelBootMode, ZtpMethod,
};

use super::helpers::{EnumColumn, enum_column, rule_triggers};

const TABLE: &str = "node_image";

pub(crate) const TABLES: &str = include_str!("../../sql/schema/node_image.sql");

/// The enum columns, and the values the current Rust enums allow in them.
pub(crate) fn columns() -> Result<Vec<EnumColumn>> {
    Ok(vec![
        enum_column(TABLE, "model", &NodeModel::to_vec(), false)?,
        enum_column(TABLE, "os_variant", &OsVariant::to_vec(), false)?,
        enum_column(TABLE, "kind", &NodeKind::to_vec(), false)?,
        enum_column(TABLE, "bios", &BiosTypes::to_vec(), false)?,
        enum_column(TABLE, "cpu_architecture", &CpuArchitecture::to_vec(), false)?,
        enum_column(TABLE, "cpu_model", &CpuModels::to_vec(), false)?,
        enum_column(TABLE, "machine_type", &MachineType::to_vec(), false)?,
        enum_column(TABLE, "hdd_bus", &DiskBuses::to_vec(), false)?,
        enum_column(TABLE, "cdrom_bus", &DiskBuses::to_vec(), false)?,
        enum_column(TABLE, "ztp_method", &ZtpMethod::to_vec(), false)?,
        enum_column(TABLE, "interface_type", &InterfaceType::to_vec(), false)?,
        enum_column(
            TABLE,
            "management_interface",
            &MgmtInterfaces::to_vec(),
            false,
        )?,
        enum_column(TABLE, "boot_mode", &UnikernelBootMode::to_vec(), true)?,
    ])
}

/// The triggers checking the enum columns against the current Rust enums.
pub(crate) fn rules() -> Result<String> {
    Ok(rule_triggers(TABLE, &columns()?))
}
