//! Mapping between the `shared::data` records and database rows.
//!
//! The mappers read and write columns in the order of each table's `CREATE TABLE` in `sql/schema/`,
//! which is the order the statements in `sql/queries/` list them. Record ids are stored as their
//! key; the table is implied by the column.
use anyhow::{Context, Result, bail};
use jiff::Timestamp;
use rand::Rng;
use serde::Serialize;
use serde::de::DeserializeOwned;
use turso::{Row, Value};

use shared::data::{
    BootDiskSettings, BridgeMember, CdromSettings, CpuSettings, DbBridge, DbLab, DbLink, DbNode,
    DbSession, DbSessionUser, DbUser, InterfaceSettings, LinkImpairment, MachineSettings,
    NodeConfig, RecordId, UnikernelSettings, ZtpSettings,
};
use shared::konst::{SHERPA_DB_KEY_CHARSET, SHERPA_DB_KEY_LENGTH};

pub(crate) const USER_TABLE: &str = "user";
pub(crate) const LAB_TABLE: &str = "lab";
pub(crate) const NODE_TABLE: &str = "node";
pub(crate) const NODE_IMAGE_TABLE: &str = "node_image";
pub(crate) const LINK_TABLE: &str = "link";
pub(crate) const BRIDGE_TABLE: &str = "bridge";

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

    /// A percentage, 0 to 100. Percentages are `f64` in Sherpa, the width of a `REAL`, so the
    /// value reads back exactly as it was written. A value Sherpa could not have written, such
    /// as one edited by hand to `NaN`, infinity or above 100, is an error rather than an
    /// impairment netem would clamp.
    pub(crate) fn percent(&mut self, field: &str) -> Result<f64> {
        match self.next(field)? {
            Value::Real(value) if (0.0..=100.0).contains(&value) => Ok(value),
            Value::Real(value) => {
                bail!("Database field {field} is not a percentage between 0 and 100: {value}")
            }
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

    /// A timestamp stored as whole Unix seconds.
    pub(crate) fn unix_seconds(&mut self, field: &str) -> Result<Timestamp> {
        let seconds: i64 = self.integer(field)?;
        Timestamp::from_second(seconds)
            .with_context(|| format!("Database field {field} is not a valid time: {seconds}"))
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

pub(crate) fn session_from_row(row: &Row) -> Result<DbSession> {
    session_from_cells(&mut Cells::new(row))
}

fn session_from_cells(cells: &mut Cells) -> Result<DbSession> {
    Ok(DbSession {
        id: cells.text("id")?,
        user: cells.record(USER_TABLE, "user")?,
        channel: cells.enumeration("channel")?,
        created_at: cells.timestamp("created_at")?,
        expires_at: cells.unix_seconds("expires_at")?,
    })
}

/// A session row followed by its user's `username` and `is_admin`.
pub(crate) fn session_user_from_row(row: &Row) -> Result<DbSessionUser> {
    let mut cells = Cells::new(row);
    Ok(DbSessionUser {
        session: session_from_cells(&mut cells)?,
        username: cells.text("username")?,
        is_admin: cells.boolean("is_admin")?,
    })
}

/// A session's column values in the `session` table's column order.
pub(crate) fn session_values(session: &DbSession) -> Result<[Value; 5]> {
    Ok([
        session.id.as_str().into(),
        key_of(&session.user, USER_TABLE)?.into(),
        enum_text(&session.channel, "channel")?.into(),
        session.created_at.to_string().into(),
        session.expires_at.as_second().into(),
    ])
}

/// A user's column values in the `user` table's column order, under `key`.
pub(crate) fn user_values(user: &DbUser, key: &str) -> Result<[Value; 7]> {
    Ok([
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
        client_host: cells.optional_text("client_host")?,
        client_folder: cells.optional_text("client_folder")?,
        description: cells.optional_text("description")?,
        phase: cells.enumeration("phase")?,
    })
}

/// A lab's column values in the `lab` table's column order, under `key`.
pub(crate) fn lab_values(lab: &DbLab, key: &str) -> Result<[Value; 17]> {
    Ok([
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
        lab.client_host.as_deref().into(),
        lab.client_folder.as_deref().into(),
        lab.description.as_deref().into(),
        enum_text(&lab.phase, "phase")?.into(),
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

/// A node's column values in the `node` table's column order, under `key`.
pub(crate) fn node_values(node: &DbNode, key: &str) -> Result<[Value; 15]> {
    Ok([
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
        loss_percent: cells.percent(&format!("{direction}.loss_percent"))?,
        reorder_percent: cells.percent(&format!("{direction}.reorder_percent"))?,
        corrupt_percent: cells.percent(&format!("{direction}.corrupt_percent"))?,
    })
}

pub(crate) fn impairment_values(impairment: &LinkImpairment) -> [Value; 5] {
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

/// A link's column values in the `link` table's column order, under `key`.
pub(crate) fn link_values(link: &DbLink, key: &str) -> Result<[Value; 20]> {
    let [a_delay, a_jitter, a_loss, a_reorder, a_corrupt] =
        impairment_values(&link.impairment_a_to_b);
    let [b_delay, b_jitter, b_loss, b_reorder, b_corrupt] =
        impairment_values(&link.impairment_b_to_a);
    Ok([
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
        a_delay,
        a_jitter,
        a_loss,
        a_reorder,
        a_corrupt,
        b_delay,
        b_jitter,
        b_loss,
        b_reorder,
        b_corrupt,
    ])
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

/// A bridge's column values in the `bridge` table's column order, under `key`.
pub(crate) fn bridge_values(bridge: &DbBridge, key: &str) -> Result<[Value; 5]> {
    Ok([
        key.into(),
        bridge.index.into(),
        bridge.bridge_name.as_str().into(),
        bridge.network_name.as_str().into(),
        key_of(&bridge.lab, LAB_TABLE)?.into(),
    ])
}

/// A bridge member, from a row of its node and interface.
pub(crate) fn bridge_member_from_row(row: &Row) -> Result<BridgeMember> {
    let mut cells = Cells::new(row);
    Ok(BridgeMember {
        node: cells.record(NODE_TABLE, "node")?,
        interface: cells.text("interface")?,
    })
}

/// A bridge member's column values in the `bridge_member` table's column order.
pub(crate) fn bridge_member_values(
    member: &BridgeMember,
    bridge_key: &str,
    position: usize,
) -> Result<[Value; 4]> {
    Ok([
        bridge_key.into(),
        i64::try_from(position)
            .context("Bridge member position is too large")?
            .into(),
        key_of(&member.node, NODE_TABLE)?.into(),
        member.interface.as_str().into(),
    ])
}

/// A node image from its `node_image` row. `Cells` reads the columns in the table's order,
/// which is not the order of the groups, so each column is read into a local first.
pub(crate) fn node_image_from_row(row: &Row) -> Result<NodeConfig> {
    let mut cells = Cells::new(row);
    let id = cells.record(NODE_IMAGE_TABLE, "id")?;
    let model = cells.enumeration("model")?;
    let version = cells.text("version")?;
    let repo = cells.optional_text("repo")?;
    let os_variant = cells.enumeration("os_variant")?;
    let kind = cells.enumeration("kind")?;
    let bios = cells.enumeration("bios")?;
    let cpu_count = cells.integer("cpu_count")?;
    let cpu_architecture = cells.enumeration("cpu_architecture")?;
    let cpu_model = cells.enumeration("cpu_model")?;
    let machine_type = cells.enumeration("machine_type")?;
    let vmx_enabled = cells.boolean("vmx_enabled")?;
    let memory = cells.integer("memory")?;
    let hdd_bus = cells.enumeration("hdd_bus")?;
    let cdrom = cells.optional_text("cdrom")?;
    let cdrom_bus = cells.enumeration("cdrom_bus")?;
    let ztp_username = cells.optional_text("ztp_username")?;
    let ztp_password = cells.optional_text("ztp_password")?;
    let ztp_password_auth = cells.boolean("ztp_password_auth")?;
    let interfaces = InterfaceSettings {
        data_count: cells.integer("data_interface_count")?,
        prefix: cells.text("interface_prefix")?,
        kind: cells.enumeration("interface_type")?,
        mtu: cells.integer("interface_mtu")?,
        first_index: cells.integer("first_interface_index")?,
        management: cells.enumeration("management_interface")?,
        reserved_count: cells.integer("reserved_interface_count")?,
    };
    let default = cells.boolean("default")?;
    let boot_mode = cells.optional_enumeration("boot_mode")?;
    let ztp_method = cells.enumeration("ztp")?;
    Ok(NodeConfig {
        id: Some(id),
        model,
        version,
        repo,
        os_variant,
        kind,
        default,
        memory,
        cpu: CpuSettings {
            count: cpu_count,
            architecture: cpu_architecture,
            model: cpu_model,
            vmx: vmx_enabled,
        },
        machine: MachineSettings {
            kind: machine_type,
            bios,
        },
        boot_disk: BootDiskSettings { bus: hdd_bus },
        cdrom: CdromSettings {
            image: cdrom,
            bus: cdrom_bus,
        },
        interfaces,
        ztp: ZtpSettings {
            method: ztp_method,
            username: ztp_username,
            password: ztp_password,
            password_auth: ztp_password_auth,
        },
        unikernel: boot_mode.map(|boot_mode| UnikernelSettings { boot_mode }),
    })
}

/// A node image's column values in the `node_image` table's column order, under `key`.
pub(crate) fn node_image_values(image: &NodeConfig, key: &str) -> Result<[Value; 29]> {
    Ok([
        key.into(),
        enum_text(&image.model, "model")?.into(),
        image.version.as_str().into(),
        image.repo.as_deref().into(),
        enum_text(&image.os_variant, "os_variant")?.into(),
        enum_text(&image.kind, "kind")?.into(),
        enum_text(&image.machine.bios, "bios")?.into(),
        image.cpu.count.into(),
        enum_text(&image.cpu.architecture, "cpu_architecture")?.into(),
        enum_text(&image.cpu.model, "cpu_model")?.into(),
        enum_text(&image.machine.kind, "machine_type")?.into(),
        image.cpu.vmx.into(),
        image.memory.into(),
        enum_text(&image.boot_disk.bus, "hdd_bus")?.into(),
        image.cdrom.image.as_deref().into(),
        enum_text(&image.cdrom.bus, "cdrom_bus")?.into(),
        image.ztp.username.as_deref().into(),
        image.ztp.password.as_deref().into(),
        image.ztp.password_auth.into(),
        image.interfaces.data_count.into(),
        image.interfaces.prefix.as_str().into(),
        enum_text(&image.interfaces.kind, "interface_type")?.into(),
        image.interfaces.mtu.into(),
        image.interfaces.first_index.into(),
        enum_text(&image.interfaces.management, "management_interface")?.into(),
        image.interfaces.reserved_count.into(),
        image.default.into(),
        image
            .unikernel
            .as_ref()
            .map(|unikernel| enum_text(&unikernel.boot_mode, "boot_mode"))
            .transpose()?
            .into(),
        enum_text(&image.ztp.method, "ztp")?.into(),
    ])
}

#[cfg(test)]
mod tests {
    use shared::data::{LabPhase, NodeKind, NodeModel};

    use super::*;
    use crate::open_in_memory;

    /// Read `value`, bound as the only column of a row, as the percentage `a_to_b.loss_percent`.
    async fn read_percent(value: Value) -> Result<f64> {
        let db = open_in_memory().await.unwrap();
        let conn = db.reader().await.unwrap();
        let mut rows = conn.query("SELECT ?1", [value]).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        Cells::new(&row).percent("a_to_b.loss_percent")
    }

    #[tokio::test]
    async fn percentages_from_0_to_100_read_back_exactly() {
        for value in [0.0, 1.5, 100.0, 0.100_000_001] {
            assert_eq!(read_percent(Value::Real(value)).await.unwrap(), value);
        }
    }

    #[tokio::test]
    async fn a_corrupt_or_out_of_range_percentage_is_an_error_naming_the_field() {
        for value in [
            Value::Real(f64::INFINITY),
            Value::Real(f64::NEG_INFINITY),
            Value::Real(f64::NAN),
            Value::Real(100.5),
            Value::Real(1e300),
            Value::Real(-0.5),
            Value::Null,
            Value::Text("12".to_string()),
        ] {
            let error = read_percent(value.clone()).await.unwrap_err().to_string();
            assert!(
                error.contains("a_to_b.loss_percent"),
                "{value:?} gave: {error}"
            );
        }
    }

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
        assert!(enum_text(&LabPhase::default(), "phase").is_ok());
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
