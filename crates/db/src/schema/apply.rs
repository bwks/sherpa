use anyhow::{Context, Result};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::Client;

use data::{
    BiosTypes, BridgeKind, CpuArchitecture, CpuModels, DiskBuses, InterfaceType, MachineType,
    MgmtInterfaces, NodeKind, NodeModel, OsVariant, ZtpMethod,
};

// ========================================
// Helper Functions to Generate Enum Lists
// ========================================

/// Generate a comma-separated list of quoted enum values for schema ASSERT clauses
/// Uses the to_vec() method that each enum provides
fn vec_to_str<T: std::fmt::Display>(vec: Vec<T>) -> String {
    vec.iter()
        .map(|variant| format!(r#""{}""#, variant))
        .collect::<Vec<_>>()
        .join(", ")
}

// ========================================
// Schema Generation Functions
// ========================================

/// Generate the user table schema
fn generate_user_schema() -> String {
    r#"
DEFINE TABLE user SCHEMAFULL;
DEFINE FIELD username ON TABLE user TYPE string
    ASSERT string::len($value) >= 3
    AND $value = /^[a-zA-Z0-9@._-]+$/;
DEFINE FIELD ssh_keys ON TABLE user TYPE array<string> DEFAULT [];

DEFINE INDEX unique_username
  ON TABLE user FIELDS username UNIQUE;
"#
    .to_string()
}

/// Generate the node_config table schema with enum-driven constraints
fn generate_node_config_schema() -> String {
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

DEFINE FIELD interface_count ON TABLE node_config TYPE number
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

DEFINE INDEX unique_node_config_name_kind
  ON TABLE node_config FIELDS model, kind UNIQUE;
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

/// Generate the lab table schema
fn generate_lab_schema() -> String {
    r#"
DEFINE TABLE lab SCHEMAFULL;
DEFINE FIELD lab_id ON TABLE lab TYPE string
    ASSERT string::len($value) >= 1;
DEFINE FIELD name ON TABLE lab TYPE string;
DEFINE FIELD user ON TABLE lab TYPE record<user>;

DEFINE INDEX unique_lab_id ON TABLE lab FIELDS lab_id UNIQUE;

DEFINE INDEX unique_lab_name_user
  ON TABLE lab FIELDS name, user UNIQUE;
"#
    .to_string()
}

/// Generate the node table schema
fn generate_node_schema() -> String {
    r#"
DEFINE TABLE node SCHEMAFULL;
DEFINE FIELD name ON TABLE node TYPE string;
DEFINE FIELD index ON TABLE node TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD config ON TABLE node TYPE record<node_config>;
DEFINE FIELD lab ON TABLE node TYPE record<lab>;

DEFINE INDEX unique_node_name_per_lab
  ON TABLE node FIELDS lab, name UNIQUE;

DEFINE INDEX unique_node_index_per_lab
  ON TABLE node FIELDS lab, index UNIQUE;
"#
    .to_string()
}

/// Generate the link table schema with enum-driven constraints
fn generate_link_schema() -> String {
    let bridge_kinds = vec_to_str(BridgeKind::to_vec());

    format!(
        r#"
DEFINE TABLE link SCHEMAFULL;
DEFINE FIELD index ON TABLE link TYPE number
    ASSERT $value >= 0 AND $value <= 65535 AND $value == math::floor($value);
DEFINE FIELD node_a ON TABLE link TYPE record<node>;
DEFINE FIELD node_b ON TABLE link TYPE record<node>;
DEFINE FIELD int_a ON TABLE link TYPE string;
DEFINE FIELD int_b ON TABLE link TYPE string;
DEFINE FIELD bridge_a ON TABLE link TYPE string;
DEFINE FIELD bridge_b ON TABLE link TYPE string;
DEFINE FIELD veth_a ON TABLE link TYPE string;
DEFINE FIELD veth_b ON TABLE link TYPE string;
DEFINE FIELD kind ON TABLE link TYPE string
    ASSERT $value IN [{}];
DEFINE FIELD lab ON TABLE link TYPE record<lab>;

DEFINE INDEX unique_peers_on_link
  ON TABLE link FIELDS node_a, node_b, int_a, int_b UNIQUE;
"#,
        bridge_kinds
    )
}

// ========================================
// Schema Application Functions
// ========================================

/// Apply a single schema section to the database
async fn apply_schema_section(
    db: &Surreal<Client>,
    section_name: &str,
    schema: &str,
) -> Result<()> {
    println!("Creating table: {}", section_name);

    db.query(schema)
        .await
        .context(format!("Failed to apply schema: {}", section_name))?;

    println!("Table created: {}", section_name);
    Ok(())
}

/// Apply all database schemas in the correct dependency order
///
/// This function creates all tables, fields, indexes, and constraints
/// required by the Sherpa application. It is idempotent - safe to run
/// multiple times without errors.
///
/// The schema constraints are automatically generated from Rust enums,
/// ensuring type safety and preventing enum variant mismatches between
/// the database schema and application code.
///
/// # Order of Execution
///
/// Tables are created in dependency order to satisfy foreign key relationships:
/// 1. user (no dependencies)
/// 2. node_config (no dependencies)
/// 3. lab (depends on: user)
/// 4. node (depends on: node_config, lab)
/// 5. link (depends on: node, lab)
///
/// # Examples
///
/// ```no_run
/// use db::{connect, apply_schema};
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let db = connect("localhost", 8000, "test", "test").await?;
///     apply_schema(&db).await?;
///     Ok(())
/// }
/// ```
pub async fn apply_schema(db: &Surreal<Client>) -> Result<()> {
    // Generate schemas dynamically from Rust enums
    let user_schema = generate_user_schema();
    let node_config_schema = generate_node_config_schema();
    let lab_schema = generate_lab_schema();
    let node_schema = generate_node_schema();
    let link_schema = generate_link_schema();

    // Apply schemas in dependency order
    apply_schema_section(db, "user", &user_schema).await?;
    apply_schema_section(db, "node_config", &node_config_schema).await?;
    apply_schema_section(db, "lab", &lab_schema).await?;
    apply_schema_section(db, "node", &node_schema).await?;
    apply_schema_section(db, "link", &link_schema).await?;

    Ok(())
}
