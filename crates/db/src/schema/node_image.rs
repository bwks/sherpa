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

pub(crate) const TABLES: &str = "
CREATE TABLE node_image (
    id TEXT PRIMARY KEY,
    model TEXT NOT NULL,
    version TEXT NOT NULL,
    repo TEXT,
    os_variant TEXT NOT NULL,
    kind TEXT NOT NULL,
    bios TEXT NOT NULL,
    cpu_count INTEGER NOT NULL CHECK (cpu_count BETWEEN 1 AND 255),
    cpu_architecture TEXT NOT NULL,
    cpu_model TEXT NOT NULL,
    machine_type TEXT NOT NULL,
    vmx_enabled INTEGER NOT NULL CHECK (vmx_enabled IN (0, 1)),
    memory INTEGER NOT NULL CHECK (memory BETWEEN 64 AND 65535),
    hdd_bus TEXT NOT NULL,
    cdrom TEXT,
    cdrom_bus TEXT NOT NULL,
    ztp_enable INTEGER NOT NULL CHECK (ztp_enable IN (0, 1)),
    ztp_method TEXT NOT NULL,
    ztp_username TEXT,
    ztp_password TEXT,
    ztp_password_auth INTEGER NOT NULL CHECK (ztp_password_auth IN (0, 1)),
    data_interface_count INTEGER NOT NULL CHECK (data_interface_count BETWEEN 0 AND 255),
    interface_prefix TEXT NOT NULL,
    interface_type TEXT NOT NULL,
    interface_mtu INTEGER NOT NULL CHECK (interface_mtu BETWEEN 576 AND 9600),
    first_interface_index INTEGER NOT NULL CHECK (first_interface_index BETWEEN 0 AND 255),
    dedicated_management_interface INTEGER NOT NULL
        CHECK (dedicated_management_interface IN (0, 1)),
    management_interface TEXT NOT NULL,
    reserved_interface_count INTEGER NOT NULL
        CHECK (reserved_interface_count BETWEEN 0 AND 255),
    \"default\" INTEGER NOT NULL CHECK (\"default\" IN (0, 1)),
    boot_mode TEXT,
    UNIQUE (model, kind, version)
) STRICT;
";

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
