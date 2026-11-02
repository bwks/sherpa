//! Mapping between the `shared::data` records and database rows.
//!
//! Each table has a column-list macro, so a query and the mapper that reads its rows always agree
//! on the column order. Record ids are stored as their key; the table is implied by the column.
use anyhow::{Context, Result, bail};
use jiff::Timestamp;
use rand::Rng;
use serde::Serialize;
use serde::de::DeserializeOwned;
use turso::{Row, Value};

use shared::data::{
    BridgeMember, DbBridge, DbLab, DbLink, DbNode, DbUser, LinkImpairment, NodeConfig, RecordId,
};
use shared::konst::{SHERPA_DB_KEY_CHARSET, SHERPA_DB_KEY_LENGTH};

pub(crate) const USER_TABLE: &str = "user";
pub(crate) const LAB_TABLE: &str = "lab";
pub(crate) const NODE_TABLE: &str = "node";
pub(crate) const NODE_IMAGE_TABLE: &str = "node_image";
pub(crate) const LINK_TABLE: &str = "link";
pub(crate) const BRIDGE_TABLE: &str = "bridge";

macro_rules! user_columns {
    () => {
        "id, username, password_hash, is_admin, ssh_keys, created_at, updated_at"
    };
}

macro_rules! lab_columns {
    () => {
        "id, lab_id, name, user, loopback_network, management_network, gateway_ipv4, \
         router_ipv4, tailscale_container_id, management_network_v6, gateway_ipv6, router_ipv6, \
         loopback_network_v6, status"
    };
}

macro_rules! node_columns {
    () => {
        "id, name, \"index\", image, lab, mgmt_ipv4, mgmt_ipv6, mgmt_mac, state, cpu_count, \
         memory, vnc_port, ssh_host_key, cpu_pinning, qemu_thread_cpus"
    };
}

macro_rules! link_columns {
    () => {
        "id, \"index\", kind, node_a, node_b, int_a, int_b, lab, tap_a, tap_b, \
         a_to_b_delay_us, a_to_b_jitter_us, a_to_b_loss_percent, a_to_b_reorder_percent, \
         a_to_b_corrupt_percent, b_to_a_delay_us, b_to_a_jitter_us, b_to_a_loss_percent, \
         b_to_a_reorder_percent, b_to_a_corrupt_percent"
    };
}

macro_rules! bridge_columns {
    () => {
        "id, \"index\", bridge_name, network_name, lab"
    };
}

macro_rules! bridge_member_columns {
    () => {
        "bridge, position, node, interface"
    };
}

macro_rules! node_image_columns {
    () => {
        "id, model, version, repo, os_variant, kind, bios, cpu_count, cpu_architecture, \
         cpu_model, machine_type, vmx_enabled, memory, hdd_bus, cdrom, cdrom_bus, ztp_enable, \
         ztp_method, ztp_username, ztp_password, ztp_password_auth, data_interface_count, \
         interface_prefix, interface_type, interface_mtu, first_interface_index, \
         dedicated_management_interface, management_interface, reserved_interface_count, \
         \"default\", boot_mode"
    };
}

pub(crate) use {
    bridge_columns, bridge_member_columns, lab_columns, link_columns, node_columns,
    node_image_columns, user_columns,
};

/// Reads a row's columns in order, naming each field in errors.
pub(crate) struct Cells<'a> {
    row: &'a Row,
    index: usize,
}

impl<'a> Cells<'a> {
    pub(crate) fn new(row: &'a Row) -> Self {
        Self { row, index: 0 }
    }

    fn next(&mut self, field: &str) -> Result<Value> {
        let value = self
            .row
            .get_value(self.index)
            .with_context(|| format!("Database row has no column for field {field}"))?;
        self.index += 1;
        Ok(value)
    }

    pub(crate) fn text(&mut self, field: &str) -> Result<String> {
        match self.next(field)? {
            Value::Text(text) => Ok(text),
            other => bail!("Database field {field} is not text: {other:?}"),
        }
    }

    pub(crate) fn optional_text(&mut self, field: &str) -> Result<Option<String>> {
        match self.next(field)? {
            Value::Null => Ok(None),
            Value::Text(text) => Ok(Some(text)),
            other => bail!("Database field {field} is not text: {other:?}"),
        }
    }

    pub(crate) fn integer<T>(&mut self, field: &str) -> Result<T>
    where
        T: TryFrom<i64>,
        T::Error: std::error::Error + Send + Sync + 'static,
    {
        match self.next(field)? {
            Value::Integer(value) => T::try_from(value)
                .with_context(|| format!("Database field {field} is out of range: {value}")),
            other => bail!("Database field {field} is not an integer: {other:?}"),
        }
    }

    pub(crate) fn optional_integer<T>(&mut self, field: &str) -> Result<Option<T>>
    where
        T: TryFrom<i64>,
        T::Error: std::error::Error + Send + Sync + 'static,
    {
        match self.next(field)? {
            Value::Null => Ok(None),
            Value::Integer(value) => T::try_from(value)
                .map(Some)
                .with_context(|| format!("Database field {field} is out of range: {value}")),
            other => bail!("Database field {field} is not an integer: {other:?}"),
        }
    }

    /// Percentages are `f32` in Sherpa and stored exactly as `REAL`.
    pub(crate) fn real(&mut self, field: &str) -> Result<f32> {
        match self.next(field)? {
            Value::Real(value) => Ok(value as f32),
            Value::Integer(value) => Ok(value as f32),
            other => bail!("Database field {field} is not a number: {other:?}"),
        }
    }

    pub(crate) fn boolean(&mut self, field: &str) -> Result<bool> {
        match self.next(field)? {
            Value::Integer(0) => Ok(false),
            Value::Integer(1) => Ok(true),
            other => bail!("Database field {field} is not a boolean: {other:?}"),
        }
    }

    pub(crate) fn json<T: DeserializeOwned>(&mut self, field: &str) -> Result<T> {
        let text = self.text(field)?;
        serde_json::from_str(&text)
            .with_context(|| format!("Failed to decode database field {field}"))
    }

    pub(crate) fn optional_json<T: DeserializeOwned>(&mut self, field: &str) -> Result<Option<T>> {
        match self.optional_text(field)? {
            Some(text) => serde_json::from_str(&text)
                .map(Some)
                .with_context(|| format!("Failed to decode database field {field}")),
            None => Ok(None),
        }
    }

    pub(crate) fn enumeration<T: DeserializeOwned>(&mut self, field: &str) -> Result<T> {
        let text = self.text(field)?;
        decode_enum(text, field)
    }

    pub(crate) fn optional_enumeration<T: DeserializeOwned>(
        &mut self,
        field: &str,
    ) -> Result<Option<T>> {
        match self.optional_text(field)? {
            Some(text) => decode_enum(text, field).map(Some),
            None => Ok(None),
        }
    }

    pub(crate) fn timestamp(&mut self, field: &str) -> Result<Timestamp> {
        self.text(field)?
            .parse()
            .with_context(|| format!("Failed to decode database timestamp field {field}"))
    }

    /// A record id stored as its key in a column that refers to `table`.
    pub(crate) fn record(&mut self, table: &str, field: &str) -> Result<RecordId> {
        Ok(RecordId::new(table, self.text(field)?))
    }
}

fn decode_enum<T: DeserializeOwned>(text: String, field: &str) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(text))
        .with_context(|| format!("Failed to decode database field {field}"))
}

/// The text a unit enum variant is stored as: its serde name.
pub(crate) fn enum_text<T: Serialize>(value: &T, field: &str) -> Result<String> {
    match serde_json::to_value(value)
        .with_context(|| format!("Failed to encode database field {field}"))?
    {
        serde_json::Value::String(text) => Ok(text),
        other => bail!("Database field {field} does not encode as text: {other}"),
    }
}

pub(crate) fn json_text<T: Serialize>(value: &T, field: &str) -> Result<String> {
    serde_json::to_string(value).with_context(|| format!("Failed to encode database field {field}"))
}

/// A new random record key.
pub(crate) fn new_key() -> String {
    let mut rng = rand::thread_rng();
    (0..SHERPA_DB_KEY_LENGTH)
        .map(|_| char::from(SHERPA_DB_KEY_CHARSET[rng.gen_range(0..SHERPA_DB_KEY_CHARSET.len())]))
        .collect()
}

/// The stored key of `id`, which must belong to `table`.
pub(crate) fn key_of(id: &RecordId, table: &str) -> Result<String> {
    if id.table != table {
        bail!("Record {id} is not a {table} record");
    }
    Ok(id.key.to_string())
}

pub(crate) fn user_from_row(row: &Row) -> Result<DbUser> {
    let mut cells = Cells::new(row);
    Ok(DbUser {
        id: Some(cells.record(USER_TABLE, "id")?),
        username: cells.text("username")?,
        password_hash: cells.text("password_hash")?,
        is_admin: cells.boolean("is_admin")?,
        ssh_keys: cells.json("ssh_keys")?,
        created_at: cells.timestamp("created_at")?,
        updated_at: cells.timestamp("updated_at")?,
    })
}

/// A user's column values in `user_columns!` order, under `key`.
pub(crate) fn user_values(user: &DbUser, key: &str) -> Result<Vec<Value>> {
    Ok(vec![
        key.into(),
        user.username.as_str().into(),
        user.password_hash.as_str().into(),
        user.is_admin.into(),
        json_text(&user.ssh_keys, "ssh_keys")?.into(),
        user.created_at.to_string().into(),
        user.updated_at.to_string().into(),
    ])
}

pub(crate) fn lab_from_row(row: &Row) -> Result<DbLab> {
    let mut cells = Cells::new(row);
    Ok(DbLab {
        id: Some(cells.record(LAB_TABLE, "id")?),
        lab_id: cells.text("lab_id")?,
        name: cells.text("name")?,
        user: cells.record(USER_TABLE, "user")?,
        loopback_network: cells.text("loopback_network")?,
        management_network: cells.text("management_network")?,
        gateway_ipv4: cells.text("gateway_ipv4")?,
        router_ipv4: cells.text("router_ipv4")?,
        tailscale_container_id: cells.optional_text("tailscale_container_id")?,
        management_network_v6: cells.optional_text("management_network_v6")?,
        gateway_ipv6: cells.optional_text("gateway_ipv6")?,
        router_ipv6: cells.optional_text("router_ipv6")?,
        loopback_network_v6: cells.optional_text("loopback_network_v6")?,
        status: cells.enumeration("status")?,
    })
}

/// A lab's column values in `lab_columns!` order, under `key`.
pub(crate) fn lab_values(lab: &DbLab, key: &str) -> Result<Vec<Value>> {
    Ok(vec![
        key.into(),
        lab.lab_id.as_str().into(),
        lab.name.as_str().into(),
        key_of(&lab.user, USER_TABLE)?.into(),
        lab.loopback_network.as_str().into(),
        lab.management_network.as_str().into(),
        lab.gateway_ipv4.as_str().into(),
        lab.router_ipv4.as_str().into(),
        lab.tailscale_container_id.as_deref().into(),
        lab.management_network_v6.as_deref().into(),
        lab.gateway_ipv6.as_deref().into(),
        lab.router_ipv6.as_deref().into(),
        lab.loopback_network_v6.as_deref().into(),
        enum_text(&lab.status, "status")?.into(),
    ])
}

pub(crate) fn node_from_row(row: &Row) -> Result<DbNode> {
    let mut cells = Cells::new(row);
    Ok(DbNode {
        id: Some(cells.record(NODE_TABLE, "id")?),
        name: cells.text("name")?,
        index: cells.integer("index")?,
        image: cells.record(NODE_IMAGE_TABLE, "image")?,
        lab: cells.record(LAB_TABLE, "lab")?,
        mgmt_ipv4: cells.optional_text("mgmt_ipv4")?,
        mgmt_ipv6: cells.optional_text("mgmt_ipv6")?,
        mgmt_mac: cells.optional_text("mgmt_mac")?,
        state: cells.enumeration("state")?,
        cpu_count: cells.optional_integer("cpu_count")?,
        memory: cells.optional_integer("memory")?,
        vnc_port: cells.optional_integer("vnc_port")?,
        ssh_host_key: cells.optional_text("ssh_host_key")?,
        cpu_pinning: cells.optional_json("cpu_pinning")?,
        qemu_thread_cpus: cells.optional_json("qemu_thread_cpus")?,
    })
}

/// A node's column values in `node_columns!` order, under `key`.
pub(crate) fn node_values(node: &DbNode, key: &str) -> Result<Vec<Value>> {
    Ok(vec![
        key.into(),
        node.name.as_str().into(),
        node.index.into(),
        key_of(&node.image, NODE_IMAGE_TABLE)?.into(),
        key_of(&node.lab, LAB_TABLE)?.into(),
        node.mgmt_ipv4.as_deref().into(),
        node.mgmt_ipv6.as_deref().into(),
        node.mgmt_mac.as_deref().into(),
        enum_text(&node.state, "state")?.into(),
        node.cpu_count.into(),
        node.memory.into(),
        node.vnc_port.into(),
        node.ssh_host_key.as_deref().into(),
        node.cpu_pinning
            .as_ref()
            .map(|cpus| json_text(cpus, "cpu_pinning"))
            .transpose()?
            .into(),
        node.qemu_thread_cpus
            .as_ref()
            .map(|cpus| json_text(cpus, "qemu_thread_cpus"))
            .transpose()?
            .into(),
    ])
}

fn impairment_from_cells(cells: &mut Cells<'_>, direction: &str) -> Result<LinkImpairment> {
    Ok(LinkImpairment {
        delay_us: cells.integer(&format!("{direction}.delay_us"))?,
        jitter_us: cells.integer(&format!("{direction}.jitter_us"))?,
        loss_percent: cells.real(&format!("{direction}.loss_percent"))?,
        reorder_percent: cells.real(&format!("{direction}.reorder_percent"))?,
        corrupt_percent: cells.real(&format!("{direction}.corrupt_percent"))?,
    })
}

fn impairment_values(impairment: &LinkImpairment) -> [Value; 5] {
    [
        impairment.delay_us.into(),
        impairment.jitter_us.into(),
        impairment.loss_percent.into(),
        impairment.reorder_percent.into(),
        impairment.corrupt_percent.into(),
    ]
}

pub(crate) fn link_from_row(row: &Row) -> Result<DbLink> {
    let mut cells = Cells::new(row);
    Ok(DbLink {
        id: Some(cells.record(LINK_TABLE, "id")?),
        index: cells.integer("index")?,
        kind: cells.enumeration("kind")?,
        node_a: cells.record(NODE_TABLE, "node_a")?,
        node_b: cells.record(NODE_TABLE, "node_b")?,
        int_a: cells.text("int_a")?,
        int_b: cells.text("int_b")?,
        lab: cells.record(LAB_TABLE, "lab")?,
        tap_a: cells.text("tap_a")?,
        tap_b: cells.text("tap_b")?,
        impairment_a_to_b: impairment_from_cells(&mut cells, "impairment_a_to_b")?,
        impairment_b_to_a: impairment_from_cells(&mut cells, "impairment_b_to_a")?,
    })
}

/// A link's column values in `link_columns!` order, under `key`.
pub(crate) fn link_values(link: &DbLink, key: &str) -> Result<Vec<Value>> {
    let mut values = vec![
        key.into(),
        link.index.into(),
        enum_text(&link.kind, "kind")?.into(),
        key_of(&link.node_a, NODE_TABLE)?.into(),
        key_of(&link.node_b, NODE_TABLE)?.into(),
        link.int_a.as_str().into(),
        link.int_b.as_str().into(),
        key_of(&link.lab, LAB_TABLE)?.into(),
        link.tap_a.as_str().into(),
        link.tap_b.as_str().into(),
    ];
    values.extend(impairment_values(&link.impairment_a_to_b));
    values.extend(impairment_values(&link.impairment_b_to_a));
    Ok(values)
}

/// A bridge without its members, which live in their own table.
pub(crate) fn bridge_from_row(row: &Row) -> Result<DbBridge> {
    let mut cells = Cells::new(row);
    Ok(DbBridge {
        id: Some(cells.record(BRIDGE_TABLE, "id")?),
        index: cells.integer("index")?,
        bridge_name: cells.text("bridge_name")?,
        network_name: cells.text("network_name")?,
        lab: cells.record(LAB_TABLE, "lab")?,
        members: Vec::new(),
    })
}

/// A bridge's column values in `bridge_columns!` order, under `key`.
pub(crate) fn bridge_values(bridge: &DbBridge, key: &str) -> Result<Vec<Value>> {
    Ok(vec![
        key.into(),
        bridge.index.into(),
        bridge.bridge_name.as_str().into(),
        bridge.network_name.as_str().into(),
        key_of(&bridge.lab, LAB_TABLE)?.into(),
    ])
}

/// A bridge member, from a row of `bridge_member_columns!` without the bridge and position.
pub(crate) fn bridge_member_from_row(row: &Row) -> Result<BridgeMember> {
    let mut cells = Cells::new(row);
    Ok(BridgeMember {
        node: cells.record(NODE_TABLE, "node")?,
        interface: cells.text("interface")?,
    })
}

/// A bridge member's column values in `bridge_member_columns!` order.
pub(crate) fn bridge_member_values(
    member: &BridgeMember,
    bridge_key: &str,
    position: usize,
) -> Result<Vec<Value>> {
    Ok(vec![
        bridge_key.into(),
        i64::try_from(position)
            .context("Bridge member position is too large")?
            .into(),
        key_of(&member.node, NODE_TABLE)?.into(),
        member.interface.as_str().into(),
    ])
}

pub(crate) fn node_image_from_row(row: &Row) -> Result<NodeConfig> {
    let mut cells = Cells::new(row);
    Ok(NodeConfig {
        id: Some(cells.record(NODE_IMAGE_TABLE, "id")?),
        model: cells.enumeration("model")?,
        version: cells.text("version")?,
        repo: cells.optional_text("repo")?,
        os_variant: cells.enumeration("os_variant")?,
        kind: cells.enumeration("kind")?,
        bios: cells.enumeration("bios")?,
        cpu_count: cells.integer("cpu_count")?,
        cpu_architecture: cells.enumeration("cpu_architecture")?,
        cpu_model: cells.enumeration("cpu_model")?,
        machine_type: cells.enumeration("machine_type")?,
        vmx_enabled: cells.boolean("vmx_enabled")?,
        memory: cells.integer("memory")?,
        hdd_bus: cells.enumeration("hdd_bus")?,
        cdrom: cells.optional_text("cdrom")?,
        cdrom_bus: cells.enumeration("cdrom_bus")?,
        ztp_enable: cells.boolean("ztp_enable")?,
        ztp_method: cells.enumeration("ztp_method")?,
        ztp_username: cells.optional_text("ztp_username")?,
        ztp_password: cells.optional_text("ztp_password")?,
        ztp_password_auth: cells.boolean("ztp_password_auth")?,
        data_interface_count: cells.integer("data_interface_count")?,
        interface_prefix: cells.text("interface_prefix")?,
        interface_type: cells.enumeration("interface_type")?,
        interface_mtu: cells.integer("interface_mtu")?,
        first_interface_index: cells.integer("first_interface_index")?,
        dedicated_management_interface: cells.boolean("dedicated_management_interface")?,
        management_interface: cells.enumeration("management_interface")?,
        reserved_interface_count: cells.integer("reserved_interface_count")?,
        default: cells.boolean("default")?,
        boot_mode: cells.optional_enumeration("boot_mode")?,
    })
}

/// A node image's column values in `node_image_columns!` order, under `key`.
pub(crate) fn node_image_values(image: &NodeConfig, key: &str) -> Result<Vec<Value>> {
    Ok(vec![
        key.into(),
        enum_text(&image.model, "model")?.into(),
        image.version.as_str().into(),
        image.repo.as_deref().into(),
        enum_text(&image.os_variant, "os_variant")?.into(),
        enum_text(&image.kind, "kind")?.into(),
        enum_text(&image.bios, "bios")?.into(),
        image.cpu_count.into(),
        enum_text(&image.cpu_architecture, "cpu_architecture")?.into(),
        enum_text(&image.cpu_model, "cpu_model")?.into(),
        enum_text(&image.machine_type, "machine_type")?.into(),
        image.vmx_enabled.into(),
        image.memory.into(),
        enum_text(&image.hdd_bus, "hdd_bus")?.into(),
        image.cdrom.as_deref().into(),
        enum_text(&image.cdrom_bus, "cdrom_bus")?.into(),
        image.ztp_enable.into(),
        enum_text(&image.ztp_method, "ztp_method")?.into(),
        image.ztp_username.as_deref().into(),
        image.ztp_password.as_deref().into(),
        image.ztp_password_auth.into(),
        image.data_interface_count.into(),
        image.interface_prefix.as_str().into(),
        enum_text(&image.interface_type, "interface_type")?.into(),
        image.interface_mtu.into(),
        image.first_interface_index.into(),
        image.dedicated_management_interface.into(),
        enum_text(&image.management_interface, "management_interface")?.into(),
        image.reserved_interface_count.into(),
        image.default.into(),
        image
            .boot_mode
            .as_ref()
            .map(|mode| enum_text(mode, "boot_mode"))
            .transpose()?
            .into(),
    ])
}

#[cfg(test)]
mod tests {
    use shared::data::{LabState, NodeKind, NodeModel};

    use super::*;

    #[test]
    fn new_keys_use_the_key_alphabet_and_length() {
        let key = new_key();
        assert_eq!(key.len(), SHERPA_DB_KEY_LENGTH);
        assert!(
            key.bytes()
                .all(|byte| SHERPA_DB_KEY_CHARSET.contains(&byte))
        );
        assert_ne!(key, new_key());
    }

    #[test]
    fn key_of_refuses_a_record_of_another_table() {
        let id = RecordId::new("node", "abc123");
        assert_eq!(key_of(&id, NODE_TABLE).unwrap(), "abc123");
        assert!(key_of(&id, LAB_TABLE).is_err());
    }

    #[test]
    fn enums_are_stored_by_their_serde_names() {
        assert_eq!(
            enum_text(&NodeKind::Container, "kind").unwrap(),
            serde_json::to_value(NodeKind::Container)
                .unwrap()
                .as_str()
                .unwrap()
        );
        let model = enum_text(&NodeModel::AristaCeos, "model").unwrap();
        assert_eq!(
            decode_enum::<NodeModel>(model, "model").unwrap(),
            NodeModel::AristaCeos
        );
        assert!(enum_text(&LabState::default(), "status").is_ok());
    }

    #[test]
    fn timestamps_keep_nanosecond_precision_as_text() {
        let created_at = "2026-08-17T12:34:56.123456789Z"
            .parse::<Timestamp>()
            .unwrap();
        assert_eq!(
            created_at.to_string().parse::<Timestamp>().unwrap(),
            created_at
        );
    }
}
