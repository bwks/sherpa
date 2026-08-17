use anyhow::{Context, Result, anyhow};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use shared::data::{DbBridge, DbLab, DbLink, DbNode, DbUser, NodeConfig, RecordId, RecordIdKey};
use surrealdb_types::{
    Datetime, RecordId as SurrealRecordId, RecordIdKey as SurrealRecordIdKey, SurrealValue,
};

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(crate) struct UserRow {
    pub id: Option<SurrealRecordId>,
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub ssh_keys: Vec<String>,
    pub created_at: Datetime,
    pub updated_at: Datetime,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(crate) struct LabRow {
    pub id: Option<SurrealRecordId>,
    pub lab_id: String,
    pub name: String,
    pub user: SurrealRecordId,
    pub loopback_network: String,
    pub management_network: String,
    pub gateway_ipv4: String,
    pub router_ipv4: String,
    pub management_network_v6: Option<String>,
    pub gateway_ipv6: Option<String>,
    pub router_ipv6: Option<String>,
    pub loopback_network_v6: Option<String>,
    pub status: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(crate) struct NodeRow {
    pub id: Option<SurrealRecordId>,
    pub name: String,
    pub image: SurrealRecordId,
    pub index: u16,
    pub lab: SurrealRecordId,
    pub mgmt_ipv4: Option<String>,
    pub mgmt_ipv6: Option<String>,
    pub mgmt_mac: Option<String>,
    pub state: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(crate) struct LinkRow {
    pub id: Option<SurrealRecordId>,
    pub index: u16,
    pub kind: serde_json::Value,
    pub node_a: SurrealRecordId,
    pub node_b: SurrealRecordId,
    pub int_a: String,
    pub int_b: String,
    pub lab: SurrealRecordId,
    pub bridge_a: String,
    pub bridge_b: String,
    pub veth_a: String,
    pub veth_b: String,
    pub tap_a: String,
    pub tap_b: String,
    pub delay_us: u32,
    pub jitter_us: u32,
    pub loss_percent: f32,
    pub reorder_percent: f32,
    pub corrupt_percent: f32,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(crate) struct BridgeRow {
    pub id: Option<SurrealRecordId>,
    pub index: u16,
    pub bridge_name: String,
    pub network_name: String,
    pub lab: SurrealRecordId,
    pub nodes: Vec<SurrealRecordId>,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(crate) struct NodeImageRow {
    pub id: Option<SurrealRecordId>,
    pub model: serde_json::Value,
    pub version: String,
    pub repo: Option<String>,
    pub os_variant: serde_json::Value,
    pub kind: serde_json::Value,
    pub bios: serde_json::Value,
    pub cpu_count: u8,
    pub cpu_architecture: serde_json::Value,
    pub cpu_model: serde_json::Value,
    pub machine_type: serde_json::Value,
    pub vmx_enabled: bool,
    pub memory: u16,
    pub hdd_bus: serde_json::Value,
    pub cdrom: Option<String>,
    pub cdrom_bus: serde_json::Value,
    pub ztp_enable: bool,
    pub ztp_method: serde_json::Value,
    pub ztp_username: Option<String>,
    pub ztp_password: Option<String>,
    pub ztp_password_auth: bool,
    pub data_interface_count: u8,
    pub interface_prefix: String,
    pub interface_type: serde_json::Value,
    pub interface_mtu: u16,
    pub first_interface_index: u8,
    pub dedicated_management_interface: bool,
    pub management_interface: serde_json::Value,
    pub reserved_interface_count: u8,
    pub default: bool,
    pub boot_mode: Option<serde_json::Value>,
}

pub(crate) fn to_surreal_id(id: &RecordId) -> SurrealRecordId {
    let key = match &id.key {
        RecordIdKey::Number(value) => SurrealRecordIdKey::Number(*value),
        RecordIdKey::String(value) => SurrealRecordIdKey::String(value.clone()),
    };
    SurrealRecordId::new(id.table.clone(), key)
}

pub(crate) fn from_surreal_id(id: SurrealRecordId) -> Result<RecordId> {
    let key = match id.key {
        SurrealRecordIdKey::Number(value) => RecordIdKey::Number(value),
        SurrealRecordIdKey::String(value) => RecordIdKey::String(value),
        unsupported => {
            return Err(anyhow!(
                "Unsupported SurrealDB record ID key: {unsupported:?}"
            ));
        }
    };
    Ok(RecordId::new(id.table.as_str(), key))
}

fn encode<T: Serialize>(value: T, field: &str) -> Result<serde_json::Value> {
    serde_json::to_value(value).with_context(|| format!("Failed to encode database field {field}"))
}

fn decode<T: DeserializeOwned>(value: serde_json::Value, field: &str) -> Result<T> {
    serde_json::from_value(value)
        .with_context(|| format!("Failed to decode database field {field}"))
}

fn to_datetime<T: std::fmt::Display>(value: T, field: &str) -> Result<Datetime> {
    value
        .to_string()
        .parse()
        .with_context(|| format!("Failed to encode database datetime field {field}"))
}

fn from_datetime<T: std::str::FromStr>(value: Datetime, field: &str) -> Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    value
        .to_string()
        .parse()
        .with_context(|| format!("Failed to decode database datetime field {field}"))
}

impl TryFrom<&DbUser> for UserRow {
    type Error = anyhow::Error;

    fn try_from(value: &DbUser) -> Result<Self> {
        Ok(Self {
            id: value.id.as_ref().map(to_surreal_id),
            username: value.username.clone(),
            password_hash: value.password_hash.clone(),
            is_admin: value.is_admin,
            ssh_keys: value.ssh_keys.clone(),
            created_at: to_datetime(value.created_at, "created_at")?,
            updated_at: to_datetime(value.updated_at, "updated_at")?,
        })
    }
}

impl TryFrom<UserRow> for DbUser {
    type Error = anyhow::Error;

    fn try_from(value: UserRow) -> Result<Self> {
        Ok(Self {
            id: value.id.map(from_surreal_id).transpose()?,
            username: value.username,
            password_hash: value.password_hash,
            is_admin: value.is_admin,
            ssh_keys: value.ssh_keys,
            created_at: from_datetime(value.created_at, "created_at")?,
            updated_at: from_datetime(value.updated_at, "updated_at")?,
        })
    }
}

impl TryFrom<&DbLab> for LabRow {
    type Error = anyhow::Error;

    fn try_from(value: &DbLab) -> Result<Self> {
        Ok(Self {
            id: value.id.as_ref().map(to_surreal_id),
            lab_id: value.lab_id.clone(),
            name: value.name.clone(),
            user: to_surreal_id(&value.user),
            loopback_network: value.loopback_network.clone(),
            management_network: value.management_network.clone(),
            gateway_ipv4: value.gateway_ipv4.clone(),
            router_ipv4: value.router_ipv4.clone(),
            management_network_v6: value.management_network_v6.clone(),
            gateway_ipv6: value.gateway_ipv6.clone(),
            router_ipv6: value.router_ipv6.clone(),
            loopback_network_v6: value.loopback_network_v6.clone(),
            status: encode(value.status, "status")?,
        })
    }
}

impl TryFrom<LabRow> for DbLab {
    type Error = anyhow::Error;

    fn try_from(value: LabRow) -> Result<Self> {
        Ok(Self {
            id: value.id.map(from_surreal_id).transpose()?,
            lab_id: value.lab_id,
            name: value.name,
            user: from_surreal_id(value.user)?,
            loopback_network: value.loopback_network,
            management_network: value.management_network,
            gateway_ipv4: value.gateway_ipv4,
            router_ipv4: value.router_ipv4,
            management_network_v6: value.management_network_v6,
            gateway_ipv6: value.gateway_ipv6,
            router_ipv6: value.router_ipv6,
            loopback_network_v6: value.loopback_network_v6,
            status: decode(value.status, "status")?,
        })
    }
}

impl TryFrom<&DbNode> for NodeRow {
    type Error = anyhow::Error;

    fn try_from(value: &DbNode) -> Result<Self> {
        Ok(Self {
            id: value.id.as_ref().map(to_surreal_id),
            name: value.name.clone(),
            image: to_surreal_id(&value.image),
            index: value.index,
            lab: to_surreal_id(&value.lab),
            mgmt_ipv4: value.mgmt_ipv4.clone(),
            mgmt_ipv6: value.mgmt_ipv6.clone(),
            mgmt_mac: value.mgmt_mac.clone(),
            state: encode(value.state, "state")?,
        })
    }
}

impl TryFrom<NodeRow> for DbNode {
    type Error = anyhow::Error;

    fn try_from(value: NodeRow) -> Result<Self> {
        Ok(Self {
            id: value.id.map(from_surreal_id).transpose()?,
            name: value.name,
            image: from_surreal_id(value.image)?,
            index: value.index,
            lab: from_surreal_id(value.lab)?,
            mgmt_ipv4: value.mgmt_ipv4,
            mgmt_ipv6: value.mgmt_ipv6,
            mgmt_mac: value.mgmt_mac,
            state: decode(value.state, "state")?,
        })
    }
}

impl TryFrom<&DbLink> for LinkRow {
    type Error = anyhow::Error;

    fn try_from(value: &DbLink) -> Result<Self> {
        Ok(Self {
            id: value.id.as_ref().map(to_surreal_id),
            index: value.index,
            kind: encode(&value.kind, "kind")?,
            node_a: to_surreal_id(&value.node_a),
            node_b: to_surreal_id(&value.node_b),
            int_a: value.int_a.clone(),
            int_b: value.int_b.clone(),
            lab: to_surreal_id(&value.lab),
            bridge_a: value.bridge_a.clone(),
            bridge_b: value.bridge_b.clone(),
            veth_a: value.veth_a.clone(),
            veth_b: value.veth_b.clone(),
            tap_a: value.tap_a.clone(),
            tap_b: value.tap_b.clone(),
            delay_us: value.delay_us,
            jitter_us: value.jitter_us,
            loss_percent: value.loss_percent,
            reorder_percent: value.reorder_percent,
            corrupt_percent: value.corrupt_percent,
        })
    }
}

impl TryFrom<LinkRow> for DbLink {
    type Error = anyhow::Error;

    fn try_from(value: LinkRow) -> Result<Self> {
        Ok(Self {
            id: value.id.map(from_surreal_id).transpose()?,
            index: value.index,
            kind: decode(value.kind, "kind")?,
            node_a: from_surreal_id(value.node_a)?,
            node_b: from_surreal_id(value.node_b)?,
            int_a: value.int_a,
            int_b: value.int_b,
            lab: from_surreal_id(value.lab)?,
            bridge_a: value.bridge_a,
            bridge_b: value.bridge_b,
            veth_a: value.veth_a,
            veth_b: value.veth_b,
            tap_a: value.tap_a,
            tap_b: value.tap_b,
            delay_us: value.delay_us,
            jitter_us: value.jitter_us,
            loss_percent: value.loss_percent,
            reorder_percent: value.reorder_percent,
            corrupt_percent: value.corrupt_percent,
        })
    }
}

impl TryFrom<&DbBridge> for BridgeRow {
    type Error = anyhow::Error;

    fn try_from(value: &DbBridge) -> Result<Self> {
        Ok(Self {
            id: value.id.as_ref().map(to_surreal_id),
            index: value.index,
            bridge_name: value.bridge_name.clone(),
            network_name: value.network_name.clone(),
            lab: to_surreal_id(&value.lab),
            nodes: value.nodes.iter().map(to_surreal_id).collect(),
        })
    }
}

impl TryFrom<BridgeRow> for DbBridge {
    type Error = anyhow::Error;

    fn try_from(value: BridgeRow) -> Result<Self> {
        Ok(Self {
            id: value.id.map(from_surreal_id).transpose()?,
            index: value.index,
            bridge_name: value.bridge_name,
            network_name: value.network_name,
            lab: from_surreal_id(value.lab)?,
            nodes: value
                .nodes
                .into_iter()
                .map(from_surreal_id)
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

impl TryFrom<&NodeConfig> for NodeImageRow {
    type Error = anyhow::Error;

    fn try_from(value: &NodeConfig) -> Result<Self> {
        Ok(Self {
            id: value.id.as_ref().map(to_surreal_id),
            model: encode(value.model, "model")?,
            version: value.version.clone(),
            repo: value.repo.clone(),
            os_variant: encode(&value.os_variant, "os_variant")?,
            kind: encode(&value.kind, "kind")?,
            bios: encode(&value.bios, "bios")?,
            cpu_count: value.cpu_count,
            cpu_architecture: encode(&value.cpu_architecture, "cpu_architecture")?,
            cpu_model: encode(&value.cpu_model, "cpu_model")?,
            machine_type: encode(&value.machine_type, "machine_type")?,
            vmx_enabled: value.vmx_enabled,
            memory: value.memory,
            hdd_bus: encode(&value.hdd_bus, "hdd_bus")?,
            cdrom: value.cdrom.clone(),
            cdrom_bus: encode(&value.cdrom_bus, "cdrom_bus")?,
            ztp_enable: value.ztp_enable,
            ztp_method: encode(&value.ztp_method, "ztp_method")?,
            ztp_username: value.ztp_username.clone(),
            ztp_password: value.ztp_password.clone(),
            ztp_password_auth: value.ztp_password_auth,
            data_interface_count: value.data_interface_count,
            interface_prefix: value.interface_prefix.clone(),
            interface_type: encode(&value.interface_type, "interface_type")?,
            interface_mtu: value.interface_mtu,
            first_interface_index: value.first_interface_index,
            dedicated_management_interface: value.dedicated_management_interface,
            management_interface: encode(&value.management_interface, "management_interface")?,
            reserved_interface_count: value.reserved_interface_count,
            default: value.default,
            boot_mode: value
                .boot_mode
                .as_ref()
                .map(|mode| encode(mode, "boot_mode"))
                .transpose()?,
        })
    }
}

impl TryFrom<NodeImageRow> for NodeConfig {
    type Error = anyhow::Error;

    fn try_from(value: NodeImageRow) -> Result<Self> {
        Ok(Self {
            id: value.id.map(from_surreal_id).transpose()?,
            model: decode(value.model, "model")?,
            version: value.version,
            repo: value.repo,
            os_variant: decode(value.os_variant, "os_variant")?,
            kind: decode(value.kind, "kind")?,
            bios: decode(value.bios, "bios")?,
            cpu_count: value.cpu_count,
            cpu_architecture: decode(value.cpu_architecture, "cpu_architecture")?,
            cpu_model: decode(value.cpu_model, "cpu_model")?,
            machine_type: decode(value.machine_type, "machine_type")?,
            vmx_enabled: value.vmx_enabled,
            memory: value.memory,
            hdd_bus: decode(value.hdd_bus, "hdd_bus")?,
            cdrom: value.cdrom,
            cdrom_bus: decode(value.cdrom_bus, "cdrom_bus")?,
            ztp_enable: value.ztp_enable,
            ztp_method: decode(value.ztp_method, "ztp_method")?,
            ztp_username: value.ztp_username,
            ztp_password: value.ztp_password,
            ztp_password_auth: value.ztp_password_auth,
            data_interface_count: value.data_interface_count,
            interface_prefix: value.interface_prefix,
            interface_type: decode(value.interface_type, "interface_type")?,
            interface_mtu: value.interface_mtu,
            first_interface_index: value.first_interface_index,
            dedicated_management_interface: value.dedicated_management_interface,
            management_interface: decode(value.management_interface, "management_interface")?,
            reserved_interface_count: value.reserved_interface_count,
            default: value.default,
            boot_mode: value
                .boot_mode
                .map(|mode| decode(mode, "boot_mode"))
                .transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use shared::data::{NodeKind, NodeModel};

    use super::*;

    #[test]
    fn record_id_round_trip_preserves_string_key() {
        let original = RecordId::new("node", "abc123");
        let converted = from_surreal_id(to_surreal_id(&original)).unwrap();

        assert_eq!(converted, original);
    }

    #[test]
    fn user_round_trip_preserves_timestamp_precision() {
        let created_at = "2026-08-17T12:34:56.123456789Z"
            .parse::<Timestamp>()
            .unwrap();
        let original = DbUser {
            id: Some(RecordId::new("user", "alice")),
            username: "alice".to_owned(),
            password_hash: "hash".to_owned(),
            is_admin: true,
            ssh_keys: Vec::new(),
            created_at,
            updated_at: created_at,
        };

        let row = UserRow::try_from(&original).unwrap();
        let converted = DbUser::try_from(row).unwrap();

        assert_eq!(converted.created_at, original.created_at);
        assert_eq!(converted.updated_at, original.updated_at);
    }

    #[test]
    fn node_image_round_trip_preserves_enum_values() {
        let original = NodeConfig {
            model: NodeModel::AristaCeos,
            kind: NodeKind::Container,
            ..NodeConfig::default()
        };
        let row = NodeImageRow::try_from(&original).unwrap();
        let converted = NodeConfig::try_from(row).unwrap();

        assert_eq!(converted.model, original.model);
        assert_eq!(converted.kind, original.kind);
    }
}
