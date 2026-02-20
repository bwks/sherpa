//! Node configuration table schema definition
//!
//! The node_config table stores hardware and software configuration templates
//! for different types of network nodes (routers, switches, servers, etc.).
//! Each configuration defines the virtual hardware specifications and boot parameters
//! for a specific node model.
//!
//! ## Fields
//! - Model identification: `model`, `version`, `repo`
//! - OS configuration: `os_variant`, `kind`, `bios`
//! - CPU configuration: `cpu_count`, `cpu_architecture`, `cpu_model`, `machine_type`, `vmx_enabled`
//! - Memory: `memory` (in MB)
//! - Storage: `hdd_bus`, `cdrom`, `cdrom_bus`
//! - Zero Touch Provisioning: `ztp_enable`, `ztp_method`, `ztp_username`, `ztp_password`, `ztp_password_auth`
//! - Network interfaces: `data_interface_count`, `interface_prefix`, `interface_type`, `interface_mtu`,
//!   `first_interface_index`, `dedicated_management_interface`, `management_interface`, `reserved_interface_count`
//! - Version control: `default` (boolean indicating if this is the default version for the model/kind)
//!
//! ## Constraints
//! - All enum fields are validated against their respective Rust enum variants
//! - Numeric fields have min/max bounds and must be integers
//! - Unique constraint on (model, kind, version) combination
//! - Only one configuration per (model, kind) can have default=true (enforced by application logic)
//!
//! ## Computed Fields
//! - `nodes`: Reverse reference to all nodes using this config (`<~(node FIELD config)`)
//!
//! ## Relationships
//! - One-to-many with `node` table (one config can be used by many nodes)

use shared::data::{
    BiosTypes, CpuArchitecture, CpuModels, DiskBuses, InterfaceType, MachineType, MgmtInterfaces,
    NodeKind, NodeModel, OsVariant, ZtpMethod,
};

use super::helpers::vec_to_str;

/// Generate the node_config table schema with enum-driven constraints.
///
/// Creates the node_config table with comprehensive validation rules that are
/// automatically derived from Rust enums. This ensures type safety between the
/// database schema and application code.
///
/// # Returns
///
/// A string containing the complete SurrealDB schema definition for the node_config table.
///
/// # Schema Details
///
/// - **Table**: `node_config` (SCHEMAFULL)
/// - **Fields**: 29 fields covering model, hardware, and network configuration
/// - **Indexes**:
///   - `unique_node_config_model_kind_version`: Ensures unique (model, kind, version) combinations
///
/// # Enum-Driven Validation
///
/// The schema uses ASSERT IN clauses populated from Rust enums:
/// - `NodeModel`: Valid node models (e.g., AristaVeos, CiscoNexus9300v)
/// - `OsVariant`: Operating system variants
/// - `NodeKind`: Node types (vm, container, unikernel)
/// - `BiosTypes`: BIOS types (bios, uefi)
/// - `CpuArchitecture`: CPU architectures (x86_64, aarch64)
/// - `CpuModels`: CPU models for emulation
/// - `MachineType`: QEMU machine types
/// - `DiskBuses`: Storage bus types (virtio, sata, ide)
/// - `ZtpMethod`: Zero-touch provisioning methods
/// - `InterfaceType`: Network interface types (virtio, e1000, etc.)
/// - `MgmtInterfaces`: Management interface types
///
/// # Examples
///
/// ```ignore
/// let schema = generate_node_config_schema();
/// db.query(&schema).await?;
/// ```
pub(crate) fn generate_node_config_schema() -> String {
    // Generate ASSERT lists from enums using their to_vec() methods
    let models = vec_to_str(NodeModel::to_vec());
    let os_variants = vec_to_str(OsVariant::to_vec());
    let kinds = vec_to_str(NodeKind::to_vec());
    let bios_types = vec_to_str(BiosTypes::to_vec());
    let cpu_archs = vec_to_str(CpuArchitecture::to_vec());
    let cpu_models = vec_to_str(CpuModels::to_vec());
    let machine_types = vec_to_str(MachineType::to_vec());
    let disk_buses = vec_to_str(DiskBuses::to_vec());
    let ztp_methods = vec_to_str(ZtpMethod::to_vec());
    let interface_types = vec_to_str(InterfaceType::to_vec());
    let mgmt_interfaces = vec_to_str(MgmtInterfaces::to_vec());

    format!(
        r#"
DEFINE TABLE node_config SCHEMAFULL;

DEFINE FIELD model ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD version ON TABLE node_config TYPE string;
DEFINE FIELD repo ON TABLE node_config TYPE option<string>;

DEFINE FIELD os_variant ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD kind ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD bios ON TABLE node_config TYPE string
    ASSERT $value IN [{}];

DEFINE FIELD cpu_count ON TABLE node_config TYPE number
    ASSERT $value >= 1 AND $value <= 255 AND $value == math::floor($value);
DEFINE FIELD cpu_architecture ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD cpu_model ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD machine_type ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD vmx_enabled ON TABLE node_config TYPE bool;

DEFINE FIELD memory ON TABLE node_config TYPE number
    ASSERT $value >= 64 AND $value <= 65535 AND $value == math::floor($value);

DEFINE FIELD hdd_bus ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD cdrom ON TABLE node_config TYPE option<string>;
DEFINE FIELD cdrom_bus ON TABLE node_config TYPE string
    ASSERT $value IN [{}];

DEFINE FIELD ztp_enable ON TABLE node_config TYPE bool;
DEFINE FIELD ztp_method ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD ztp_username ON TABLE node_config TYPE option<string>;
DEFINE FIELD ztp_password ON TABLE node_config TYPE option<string>;
DEFINE FIELD ztp_password_auth ON TABLE node_config TYPE bool;

DEFINE FIELD data_interface_count ON TABLE node_config TYPE number
    ASSERT $value >= 1 AND $value <= 255 AND $value == math::floor($value);
DEFINE FIELD interface_prefix ON TABLE node_config TYPE string;
DEFINE FIELD interface_type ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD interface_mtu ON TABLE node_config TYPE number
    ASSERT $value >= 576 AND $value <= 9600 AND $value == math::floor($value);
DEFINE FIELD first_interface_index ON TABLE node_config TYPE number
    ASSERT $value >= 0 AND $value <= 255 AND $value == math::floor($value);
DEFINE FIELD dedicated_management_interface ON TABLE node_config TYPE bool;
DEFINE FIELD management_interface ON TABLE node_config TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD reserved_interface_count ON TABLE node_config TYPE number
    ASSERT $value >= 0 AND $value <= 255 AND $value == math::floor($value);

DEFINE FIELD default ON TABLE node_config TYPE bool;

DEFINE FIELD nodes ON TABLE node_config COMPUTED <~(node FIELD config);

DEFINE INDEX unique_node_config_model_kind_version
  ON TABLE node_config FIELDS model, kind, version UNIQUE;
"#,
        models,
        os_variants,
        kinds,
        bios_types,
        cpu_archs,
        cpu_models,
        machine_types,
        disk_buses,
        disk_buses, // cdrom_bus uses same enum
        ztp_methods,
        interface_types,
        mgmt_interfaces,
    )
}
