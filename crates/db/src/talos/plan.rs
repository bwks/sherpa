//! Storing and reading the lab's Talos cluster plans.

use std::net::Ipv4Addr;

use anyhow::{Context, Result, anyhow, bail};
use tracing::instrument;
use turso::Row;

use shared::data::{AddressRange, ClusterCni, ClusterMember, ClusterPlan, RecordId};

use crate::Database;
use crate::persistence::{Cells, LAB_TABLE, key_of};
use crate::queries::talos;
use crate::sql::{execute, query_all};

/// A stored cluster: its position in the definition and its plan, with no members yet.
struct PlanRow {
    position: usize,
    plan: ClusterPlan,
}

/// A cluster member as stored: the cluster's position, its role and its node's address.
struct MemberRow {
    cluster: usize,
    role: String,
    member: ClusterMember,
}

/// Store what the server worked out for each of `plans`, clusters of the lab `lab_id`'s
/// definition, in one transaction: the endpoint, shared address and load balancer addresses.
/// The rest of a plan is the definition's and the nodes', and is not stored again. A cluster's
/// stored plan is replaced, so storing the same plans again changes nothing.
///
/// # Errors
///
/// Fails when a plan names a cluster the lab's definition does not have, or the database
/// cannot be written; nothing is stored then.
#[instrument(skip(db, plans), fields(%lab_id, clusters = plans.len()), level = "debug")]
pub async fn store_talos_cluster_plans(
    db: &Database,
    lab_id: &RecordId,
    plans: &[ClusterPlan],
) -> Result<()> {
    let lab_key = key_of(lab_id, LAB_TABLE)?;
    let mut writer = db.writer().await?;
    let tx = writer
        .conn
        .transaction()
        .await
        .context("Failed to start the Talos cluster plan transaction")?;
    for plan in plans {
        let name = plan.name.as_str();
        execute(
            &tx,
            talos::DELETE_CLUSTER_PLAN.bind([lab_key.as_str().into(), name.into()]),
        )
        .await
        .with_context(|| format!("Failed to replace the plan of Talos cluster {name}"))?;
        let range = plan.load_balancer_addresses;
        let stored = execute(
            &tx,
            talos::CREATE_CLUSTER_PLAN.bind([
                lab_key.as_str().into(),
                name.into(),
                plan.endpoint.to_string().into(),
                plan.vip.map(|vip| vip.to_string()).into(),
                range.map(|range| range.first.to_string()).into(),
                range.map(|range| range.last.to_string()).into(),
            ]),
        )
        .await
        .with_context(|| format!("Failed to store the plan of Talos cluster {name}"))?;
        if stored != 1 {
            bail!("Talos cluster {name} is not in the definition of lab {lab_id}");
        }
    }
    tx.commit()
        .await
        .context("Failed to commit the Talos cluster plans")
}

/// The plans of the lab `lab_id`'s clusters, in manifest order: the manifest's side of each
/// stored cluster, its members' and services' management addresses from their nodes, and what
/// the server worked out for it. A cluster with no stored plan is left out.
///
/// # Errors
///
/// Fails when the database cannot be read, or a member or service has no management address.
#[instrument(skip(db), fields(%lab_id), level = "debug")]
pub async fn list_talos_cluster_plans(
    db: &Database,
    lab_id: &RecordId,
) -> Result<Vec<ClusterPlan>> {
    let lab_key = key_of(lab_id, LAB_TABLE)?;
    let conn = db.reader().await?;
    let mut rows = query_all(
        &conn,
        talos::LIST_CLUSTER_PLANS.bind([lab_key.as_str().into()]),
        plan_row,
    )
    .await?;
    for row in query_all(
        &conn,
        talos::LIST_CLUSTER_MEMBERS.bind([lab_key.as_str().into()]),
        member_row,
    )
    .await?
    {
        let Some(cluster) = rows
            .iter_mut()
            .find(|cluster| cluster.position == row.cluster)
        else {
            continue;
        };
        match row.role.as_str() {
            "controller" => cluster.plan.control_planes.push(row.member),
            "worker" => cluster.plan.workers.push(row.member),
            other => bail!("Database field role is not a Talos cluster role: {other}"),
        }
    }
    Ok(rows.into_iter().map(|row| row.plan).collect())
}

fn plan_row(row: &Row) -> Result<PlanRow> {
    let mut cells = Cells::new(row);
    let position = cells.integer("position")?;
    let name = cells.text("name")?;
    let cni: ClusterCni = cells.enumeration("cni")?;
    let endpoint = cells.ipv4("endpoint")?;
    let vip = cells.optional_ipv4("vip")?;
    let first = cells.optional_ipv4("load_balancer_first")?;
    let last = cells.optional_ipv4("load_balancer_last")?;
    let load_balancer_addresses = match (first, last) {
        (Some(first), Some(last)) => Some(AddressRange { first, last }),
        (None, None) => None,
        _ => bail!("Talos cluster {name} has half a load balancer address range"),
    };
    let oidc = service(
        cells.optional_text("oidc")?,
        cells.optional_ipv4("oidc_mgmt_ipv4")?,
    )?;
    let vault = service(
        cells.optional_text("vault")?,
        cells.optional_ipv4("vault_mgmt_ipv4")?,
    )?;
    Ok(PlanRow {
        position,
        plan: ClusterPlan {
            name,
            endpoint,
            vip,
            cni,
            load_balancer_addresses,
            control_planes: Vec::new(),
            workers: Vec::new(),
            oidc,
            vault,
        },
    })
}

/// The Keycloak or Vault node `name` a cluster uses, with its management address `ipv4`, if
/// it uses one.
fn service(name: Option<String>, ipv4: Option<Ipv4Addr>) -> Result<Option<ClusterMember>> {
    match name {
        None => Ok(None),
        Some(name) => {
            let ipv4 = node_address(&name, ipv4)?;
            Ok(Some(ClusterMember { name, ipv4 }))
        }
    }
}

fn member_row(row: &Row) -> Result<MemberRow> {
    let mut cells = Cells::new(row);
    let cluster = cells.integer("cluster")?;
    let role = cells.text("role")?;
    let name = cells.text("node_name")?;
    let ipv4 = node_address(&name, cells.optional_ipv4("mgmt_ipv4")?)?;
    Ok(MemberRow {
        cluster,
        role,
        member: ClusterMember { name, ipv4 },
    })
}

/// The management address of the node `name`, which a cluster needs.
fn node_address(name: &str, ipv4: Option<Ipv4Addr>) -> Result<Ipv4Addr> {
    ipv4.ok_or_else(|| anyhow!("Node {name}, used by a Talos cluster, has no management address"))
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use shared::data::{AddressRange, ClusterCni, ClusterMember, DbLab, ModelKind, NodeModel};
    use topology::{Manifest, Node, TalosCluster};
    use turso::Value;

    use super::*;
    use crate::{
        DefaultRequest, LabDetails, LabNetworks, SecretKey, apply_schema, create_lab, create_node,
        create_user, delete_lab_from, open_in_memory, store_lab_definition, update_node_mgmt_ipv4,
        upsert_node_image,
    };

    /// The lab's nodes, with the last octet of their management addresses.
    const NODES: [(&str, NodeModel, u8); 6] = [
        ("cp1", NodeModel::TalosLinux, 21),
        ("cp2", NodeModel::TalosLinux, 22),
        ("cp3", NodeModel::TalosLinux, 23),
        ("w1", NodeModel::TalosLinux, 24),
        ("solo", NodeModel::TalosLinux, 25),
        ("kc01", NodeModel::KeycloakServer, 26),
    ];

    fn address(host: u8) -> Ipv4Addr {
        Ipv4Addr::new(172, 31, 1, host)
    }

    fn member(name: &str, host: u8) -> ClusterMember {
        ClusterMember {
            name: name.to_owned(),
            ipv4: address(host),
        }
    }

    /// The lab's definition: an HA Cilium cluster using Keycloak, and a single-node Flannel
    /// cluster.
    fn manifest() -> Manifest {
        Manifest {
            name: "core".to_owned(),
            talos_clusters: Some(vec![
                TalosCluster {
                    name: "ha".to_owned(),
                    controllers: vec!["cp1".to_owned(), "cp2".to_owned(), "cp3".to_owned()],
                    workers: vec!["w1".to_owned()],
                    cni: ClusterCni::Cilium,
                    oidc: Some("kc01".to_owned()),
                    vault: None,
                },
                TalosCluster {
                    name: "solo".to_owned(),
                    controllers: vec!["solo".to_owned()],
                    ..TalosCluster::default()
                },
            ]),
            nodes: NODES
                .iter()
                .map(|(name, model, _)| Node {
                    name: (*name).to_owned(),
                    model: *model,
                    ..Node::default()
                })
                .collect(),
            ..Manifest::default()
        }
    }

    /// The plans `up` works out for `manifest()`.
    fn plans() -> Vec<ClusterPlan> {
        vec![
            ClusterPlan {
                name: "ha".to_owned(),
                endpoint: address(11),
                vip: Some(address(11)),
                cni: ClusterCni::Cilium,
                load_balancer_addresses: Some(AddressRange {
                    first: address(13),
                    last: address(16),
                }),
                control_planes: vec![member("cp1", 21), member("cp2", 22), member("cp3", 23)],
                workers: vec![member("w1", 24)],
                oidc: Some(member("kc01", 26)),
                vault: None,
            },
            ClusterPlan {
                name: "solo".to_owned(),
                endpoint: address(25),
                vip: None,
                cni: ClusterCni::Flannel,
                load_balancer_addresses: None,
                control_planes: vec![member("solo", 25)],
                workers: vec![],
                oidc: None,
                vault: None,
            },
        ]
    }

    /// A database holding the lab `lab-0001`, its nodes with their management addresses, and
    /// `manifest()` as its definition.
    async fn lab() -> (Database, DbLab) {
        let db = open_in_memory()
            .await
            .unwrap()
            .with_secret_key(SecretKey::generate());
        apply_schema(&db).await.unwrap();
        let user = create_user(&db, "alice".to_owned(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        let lab = create_lab(
            &db,
            "core",
            "lab-0001",
            &user,
            &LabNetworks {
                management_network: "172.31.1.0/24".to_owned(),
                gateway_ipv4: "172.31.1.1".to_owned(),
                router_ipv4: "172.31.1.2".to_owned(),
                management_network_v6: None,
                loopback_network_v6: None,
                gateway_ipv6: None,
                router_ipv6: None,
            },
            &LabDetails::default(),
        )
        .await
        .unwrap();
        for (index, (name, model, host)) in (1..).zip(NODES) {
            let image = upsert_node_image(
                &db,
                ModelKind::default_for(model).node_config(),
                DefaultRequest::Keep,
            )
            .await
            .unwrap();
            let node = create_node(&db, name, index, image.id.unwrap(), lab.id.clone().unwrap())
                .await
                .unwrap();
            update_node_mgmt_ipv4(&db, node.id.unwrap(), &address(host).to_string())
                .await
                .unwrap();
        }
        store_lab_definition(&db, lab.id.as_ref().unwrap(), &manifest())
            .await
            .unwrap();
        (db, lab)
    }

    async fn plan_rows(db: &Database) -> i64 {
        let conn = db.reader().await.unwrap();
        let mut rows = conn
            .query("SELECT count(*) FROM talos_cluster_plan", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        match row.get_value(0).unwrap() {
            Value::Integer(count) => count,
            other => panic!("count is not an integer: {other:?}"),
        }
    }

    #[tokio::test]
    async fn stored_plans_read_back_as_planned() {
        let (db, lab) = lab().await;
        let lab_id = lab.id.unwrap();

        store_talos_cluster_plans(&db, &lab_id, &plans())
            .await
            .unwrap();

        assert_eq!(
            list_talos_cluster_plans(&db, &lab_id).await.unwrap(),
            plans()
        );
    }

    #[tokio::test]
    async fn a_lab_without_stored_plans_has_none() {
        let (db, lab) = lab().await;

        assert_eq!(
            list_talos_cluster_plans(&db, lab.id.as_ref().unwrap())
                .await
                .unwrap(),
            Vec::<ClusterPlan>::new()
        );
    }

    #[tokio::test]
    async fn storing_the_plans_again_replaces_them() {
        let (db, lab) = lab().await;
        let lab_id = lab.id.unwrap();
        let mut moved = plans();
        moved[0].endpoint = address(12);
        moved[0].vip = Some(address(12));
        store_talos_cluster_plans(&db, &lab_id, &moved)
            .await
            .unwrap();

        store_talos_cluster_plans(&db, &lab_id, &plans()[..1])
            .await
            .unwrap();
        store_talos_cluster_plans(&db, &lab_id, &plans()[1..])
            .await
            .unwrap();

        assert_eq!(
            list_talos_cluster_plans(&db, &lab_id).await.unwrap(),
            plans()
        );
        assert_eq!(plan_rows(&db).await, 2);
    }

    #[tokio::test]
    async fn a_plan_for_a_cluster_not_in_the_definition_stores_nothing() {
        let (db, lab) = lab().await;
        let lab_id = lab.id.unwrap();
        let mut unknown = plans();
        unknown[1].name = "gone".to_owned();

        let error = store_talos_cluster_plans(&db, &lab_id, &unknown)
            .await
            .unwrap_err();

        assert!(format!("{error:#}").contains("gone"), "{error:#}");
        assert_eq!(plan_rows(&db).await, 0);
    }

    #[tokio::test]
    async fn a_member_without_a_management_address_is_refused() {
        let (db, lab) = lab().await;
        let lab_id = lab.id.unwrap();
        store_talos_cluster_plans(&db, &lab_id, &plans())
            .await
            .unwrap();
        db.writer()
            .await
            .unwrap()
            .conn
            .execute("UPDATE node SET mgmt_ipv4 = NULL WHERE name = 'w1'", ())
            .await
            .unwrap();

        let error = list_talos_cluster_plans(&db, &lab_id).await.unwrap_err();

        assert!(format!("{error:#}").contains("w1"), "{error:#}");
    }

    #[tokio::test]
    async fn destroying_the_lab_removes_its_plans() {
        let (db, lab) = lab().await;
        store_talos_cluster_plans(&db, lab.id.as_ref().unwrap(), &plans())
            .await
            .unwrap();

        assert!(delete_lab_from(&db, &lab.lab_id, lab.phase).await.unwrap());

        assert_eq!(plan_rows(&db).await, 0);
    }
}
