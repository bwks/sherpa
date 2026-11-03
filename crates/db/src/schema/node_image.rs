//! Node image table schema
//!
//! One row per imported image version of a node model. `(model, kind, version)` is unique.
//! A node image used by a node cannot be deleted (`node.image` is `ON DELETE RESTRICT`).
use anyhow::Result;
use shared::data::{
    BiosTypes, CpuArchitecture, CpuModels, DiskBuses, InterfaceType, MachineType, MgmtInterfaces,
    NodeKind, NodeModel, OsVariant, UnikernelBootMode, ZtpMethod,
};

use super::helpers::{enum_rule, rule_triggers};

const TABLE: &str = "node_image";

pub(crate) const TABLES: &str = include_str!("../../sql/schema/node_image.sql");

/// The triggers checking the enum columns against the current Rust enums.
pub(crate) fn rules() -> Result<String> {
    let rules = [
        enum_rule(TABLE, "model", NodeModel::to_vec(), false)?,
        enum_rule(TABLE, "os_variant", OsVariant::to_vec(), false)?,
        enum_rule(TABLE, "kind", NodeKind::to_vec(), false)?,
        enum_rule(TABLE, "bios", BiosTypes::to_vec(), false)?,
        enum_rule(TABLE, "cpu_architecture", CpuArchitecture::to_vec(), false)?,
        enum_rule(TABLE, "cpu_model", CpuModels::to_vec(), false)?,
        enum_rule(TABLE, "machine_type", MachineType::to_vec(), false)?,
        enum_rule(TABLE, "hdd_bus", DiskBuses::to_vec(), false)?,
        enum_rule(TABLE, "cdrom_bus", DiskBuses::to_vec(), false)?,
        enum_rule(TABLE, "ztp_method", ZtpMethod::to_vec(), false)?,
        enum_rule(TABLE, "interface_type", InterfaceType::to_vec(), false)?,
        enum_rule(
            TABLE,
            "management_interface",
            MgmtInterfaces::to_vec(),
            false,
        )?,
        enum_rule(TABLE, "boot_mode", UnikernelBootMode::to_vec(), true)?,
    ];
    Ok(rule_triggers(TABLE, &rules))
}
