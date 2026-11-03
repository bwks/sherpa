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

/// Corrects node images written with an old default: `cisco_csr1000v` and `cisco_iosvl2` once
/// defaulted to 16 data interfaces, one more than they have after the management interface, so
/// `sherpa up` failed on them. Run as schema migration 2, and by the importer after it writes node
/// images from a server older than 0.5.0. Released migrations never change, so the model names are
/// written out rather than derived.
pub(crate) const FIX_DATA_INTERFACE_COUNTS: &str = "
UPDATE node_image SET data_interface_count = 15
WHERE model IN ('cisco_csr1000v', 'cisco_iosvl2') AND data_interface_count > 15;
";

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
