//! Rows as a pre-0.5.0 server stored them in SurrealDB, and their conversion to Sherpa's records.
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use shared::data::{
    BridgeMember, DbBridge, DbLab, DbLink, DbNode, DbUser, LinkImpairment, NodeConfig, RecordId,
    RecordIdKey,
};
use surrealdb_types::{
    Datetime, RecordId as SurrealRecordId, RecordIdKey as SurrealRecordIdKey, SurrealValue,
};

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(super) struct UserRow {
    pub id: Option<SurrealRecordId>,
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub ssh_keys: Vec<String>,
    pub created_at: Datetime,
    pub updated_at: Datetime,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(super) struct LabRow {
    pub id: Option<SurrealRecordId>,
    pub lab_id: String,
    pub name: String,
    pub user: SurrealRecordId,
    pub loopback_network: String,
    pub management_network: String,
    pub gateway_ipv4: String,
    pub router_ipv4: String,
    pub tailscale_container_id: Option<String>,
    pub management_network_v6: Option<String>,
    pub gateway_ipv6: Option<String>,
    pub router_ipv6: Option<String>,
    pub loopback_network_v6: Option<String>,
    pub status: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(super) struct NodeRow {
    pub id: Option<SurrealRecordId>,
    pub name: String,
    pub image: SurrealRecordId,
    pub index: u16,
    pub lab: SurrealRecordId,
    pub mgmt_ipv4: Option<String>,
    pub mgmt_ipv6: Option<String>,
    pub mgmt_mac: Option<String>,
    pub state: serde_json::Value,
    pub cpu_count: Option<u8>,
    pub memory: Option<u16>,
    pub vnc_port: Option<u16>,
    pub ssh_host_key: Option<String>,
    pub cpu_pinning: Option<Vec<u16>>,
    pub qemu_thread_cpus: Option<Vec<u16>>,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(super) struct LinkRow {
    pub id: Option<SurrealRecordId>,
    pub index: u16,
    pub kind: serde_json::Value,
    pub node_a: SurrealRecordId,
    pub node_b: SurrealRecordId,
    pub int_a: String,
    pub int_b: String,
    pub lab: SurrealRecordId,
    pub tap_a: String,
    pub tap_b: String,
    pub impairment_a_to_b: LinkImpairmentRow,
    pub impairment_b_to_a: LinkImpairmentRow,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(super) struct LinkImpairmentRow {
    pub delay_us: u32,
    pub jitter_us: u32,
    pub loss_percent: f32,
    pub reorder_percent: f32,
    pub corrupt_percent: f32,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(super) struct BridgeRow {
    pub id: Option<SurrealRecordId>,
    pub index: u16,
    pub bridge_name: String,
    pub network_name: String,
    pub lab: SurrealRecordId,
    pub members: Vec<BridgeMemberRow>,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(super) struct BridgeMemberRow {
    pub node: SurrealRecordId,
    pub interface: String,
}

#[derive(Clone, Debug, Deserialize, SurrealValue)]
pub(super) struct NodeImageRow {
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

pub(super) fn from_surreal_id(id: SurrealRecordId) -> Result<RecordId> {
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

fn decode<T: DeserializeOwned>(value: serde_json::Value, field: &str) -> Result<T> {
    serde_json::from_value(value)
        .with_context(|| format!("Failed to decode database field {field}"))
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
            tailscale_container_id: value.tailscale_container_id,
            management_network_v6: value.management_network_v6,
            gateway_ipv6: value.gateway_ipv6,
            router_ipv6: value.router_ipv6,
            loopback_network_v6: value.loopback_network_v6,
            status: decode(value.status, "status")?,
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
            cpu_count: value.cpu_count,
            memory: value.memory,
            vnc_port: value.vnc_port,
            ssh_host_key: value.ssh_host_key,
            cpu_pinning: value.cpu_pinning,
            qemu_thread_cpus: value.qemu_thread_cpus,
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
            tap_a: value.tap_a,
            tap_b: value.tap_b,
            impairment_a_to_b: LinkImpairment::from(value.impairment_a_to_b),
            impairment_b_to_a: LinkImpairment::from(value.impairment_b_to_a),
        })
    }
}

impl From<LinkImpairmentRow> for LinkImpairment {
    fn from(value: LinkImpairmentRow) -> Self {
        Self {
            delay_us: value.delay_us,
            jitter_us: value.jitter_us,
            loss_percent: value.loss_percent,
            reorder_percent: value.reorder_percent,
            corrupt_percent: value.corrupt_percent,
        }
    }
}

impl TryFrom<BridgeMemberRow> for BridgeMember {
    type Error = anyhow::Error;

    fn try_from(value: BridgeMemberRow) -> Result<Self> {
        Ok(Self {
            node: from_surreal_id(value.node)?,
            interface: value.interface,
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
            members: value
                .members
                .into_iter()
                .map(BridgeMember::try_from)
                .collect::<Result<Vec<_>>>()?,
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
    fn surreal_ids_convert_with_their_table_and_key() {
        let string = SurrealRecordId::new("node", SurrealRecordIdKey::String("abc123".into()));
        assert_eq!(
            from_surreal_id(string).unwrap(),
            RecordId::new("node", "abc123")
        );
        let number = SurrealRecordId::new("user", SurrealRecordIdKey::Number(7));
        assert_eq!(
            from_surreal_id(number).unwrap(),
            RecordId::new("user", 7_i64)
        );
    }

    #[test]
    fn user_row_keeps_timestamp_precision() {
        let created_at: Datetime = "2026-08-17T12:34:56.123456789Z".parse().unwrap();
        let row = UserRow {
            id: Some(SurrealRecordId::new(
                "user",
                SurrealRecordIdKey::String("alice".into()),
            )),
            username: "alice".into(),
            password_hash: "hash".into(),
            is_admin: false,
            ssh_keys: vec!["ssh-ed25519 AAAA".into()],
            created_at,
            updated_at: created_at,
        };
        let user = DbUser::try_from(row).unwrap();
        assert_eq!(user.id, Some(RecordId::new("user", "alice")));
        assert_eq!(
            user.created_at,
            "2026-08-17T12:34:56.123456789Z"
                .parse::<Timestamp>()
                .unwrap()
        );
        assert_eq!(user.ssh_keys, vec!["ssh-ed25519 AAAA".to_string()]);
    }

    #[test]
    fn node_image_row_decodes_enum_values() {
        let config = NodeConfig {
            model: NodeModel::AristaCeos,
            kind: NodeKind::Container,
            ..NodeConfig::default()
        };
        let encoded = serde_json::to_value(&config).unwrap();
        let row = NodeImageRow {
            id: Some(SurrealRecordId::new(
                "node_image",
                SurrealRecordIdKey::String("img".into()),
            )),
            model: encoded["model"].clone(),
            version: config.version.clone(),
            repo: None,
            os_variant: encoded["os_variant"].clone(),
            kind: encoded["kind"].clone(),
            bios: encoded["bios"].clone(),
            cpu_count: config.cpu_count,
            cpu_architecture: encoded["cpu_architecture"].clone(),
            cpu_model: encoded["cpu_model"].clone(),
            machine_type: encoded["machine_type"].clone(),
            vmx_enabled: config.vmx_enabled,
            memory: config.memory,
            hdd_bus: encoded["hdd_bus"].clone(),
            cdrom: None,
            cdrom_bus: encoded["cdrom_bus"].clone(),
            ztp_enable: config.ztp_enable,
            ztp_method: encoded["ztp_method"].clone(),
            ztp_username: None,
            ztp_password: None,
            ztp_password_auth: config.ztp_password_auth,
            data_interface_count: config.data_interface_count,
            interface_prefix: config.interface_prefix.clone(),
            interface_type: encoded["interface_type"].clone(),
            interface_mtu: config.interface_mtu,
            first_interface_index: config.first_interface_index,
            dedicated_management_interface: config.dedicated_management_interface,
            management_interface: encoded["management_interface"].clone(),
            reserved_interface_count: config.reserved_interface_count,
            default: config.default,
            boot_mode: None,
        };
        let converted = NodeConfig::try_from(row).unwrap();
        assert_eq!(converted.model, NodeModel::AristaCeos);
        assert_eq!(converted.kind, NodeKind::Container);
        assert_eq!(converted.id, Some(RecordId::new("node_image", "img")));
    }
}
