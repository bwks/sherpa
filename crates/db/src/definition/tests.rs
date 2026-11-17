use shared::data::{
    ClusterCni, CpuMode, CpuTopology, DbLab, DiskCache, DiskImageCloneMethod, DiskIo, LabPhase,
    ModelKind, NodeBootDisk, NodeCpu, NodeInterfaces, NodeKind, NodeModel, NodeNuma,
    NumaMemoryMode, PtxPlatform, RecordId, WatchdogAction, Ztp, ZtpServer,
};
use topology::{
    AuthorizedKeyFile, BinaryFile, Bridge, ContainerProvisioner, ContainerSecretFile, Link2,
    Manifest, ManifestImpairment, Node, NodeContainer, NodeReadyCheck, NodeSsh, NodeUnikernel,
    NodeVm, NodeZtp, Provisioner, ProvisionerFile, SshProvisioner, StartupScript, SystemdUnit,
    TailscaleConfig, TalosCluster, TextFile, VolumeMount,
};
use turso::Value;

use super::*;
use crate::persistence::{LAB_TABLE, key_of};
use crate::sql::{Query, query_count};
use crate::{
    Database, DefaultRequest, LabDetails, LabNetworks, SecretKey, apply_schema, create_lab,
    create_node, create_user, delete_lab_from, get_node_by_name_and_lab, open, open_in_memory,
    upsert_node_image,
};

const PASSWORD: &str = "ztp-Secret-42";

/// Every table the definition writes rows to, other than `lab` and `node`.
const CHILD_TABLES: [&str; 13] = [
    "manifest_link",
    "manifest_bridge",
    "manifest_bridge_link",
    "manifest_talos_cluster",
    "manifest_talos_member",
    "manifest_node_cpu",
    "manifest_node_text",
    "manifest_node_volume",
    "manifest_node_text_file",
    "manifest_node_systemd_unit",
    "manifest_node_provisioner",
    "manifest_node_provisioner_file",
    "manifest_node_provisioner_path",
];

/// A node that sets every field the manifest has, as the user writes it: none of the `*_data`
/// fields, which only the client fills in, from the files.
fn full_node() -> Node {
    Node {
        name: "dev01".to_owned(),
        model: NodeModel::UbuntuLinux,
        image: Some("example/ubuntu".to_owned()),
        version: Some("24.04".to_owned()),
        memory: Some(4096),
        cpu: NodeCpu {
            count: Some(4),
            mode: Some(CpuMode::HostPassthrough),
            topology: Some(CpuTopology {
                sockets: 1,
                cores: 2,
                threads: 2,
            }),
            pinning: Some(vec![2, 3, 4, 5]),
            emulator_pinning: Some(vec![6]),
        },
        numa: NodeNuma {
            node: Some(1),
            memory_mode: Some(NumaMemoryMode::Preferred),
        },
        boot_disk: NodeBootDisk {
            size: Some(40),
            clone_method: Some(DiskImageCloneMethod::Copy),
            cache: Some(DiskCache::Writeback),
            io: Some(DiskIo::IoUring),
            io_thread_cpus: Some(vec![7, 8]),
        },
        interfaces: NodeInterfaces {
            data_count: Some(8),
            queues: Some(2),
        },
        ztp: NodeZtp {
            method: Some(Ztp::CloudInit),
            config: Some("configs/dev01.yaml".to_owned()),
        },
        ssh: NodeSsh {
            authorized_keys: Some(vec![
                "ssh-ed25519 AAAA1 alice".to_owned(),
                "ssh-ed25519 AAAA2 bob".to_owned(),
            ]),
            authorized_key_files: Some(vec![AuthorizedKeyFile {
                source: "~/.ssh/id_ed25519.pub".to_owned(),
            }]),
        },
        ready_check: NodeReadyCheck {
            port: Some(2222),
            skip: Some(false),
        },
        vm: Some(NodeVm {
            watchdog: Some(WatchdogAction::InjectNmi),
        }),
        container: Some(NodeContainer {
            user: Some("1000:1000".to_owned()),
            privileged: Some(true),
            shm_size: Some(1_073_741_824),
            commands: Some(vec!["sleep".to_owned(), "infinity".to_owned()]),
            volumes: Some(vec![VolumeMount {
                src: "/srv/data".to_owned(),
                dst: "/data".to_owned(),
            }]),
            platform: Some(PtxPlatform::Bx),
        }),
        unikernel: Some(NodeUnikernel {
            cmdline: Some("console=ttyS0".to_owned()),
        }),
        ..full_node_files()
    }
}

/// The files, scripts and provisioners of [`full_node`], as the manifest names them.
fn full_node_files() -> Node {
    Node {
        environment_variables: Some(vec!["MODE=lab".to_owned(), "TOKEN=$LAB_TOKEN".to_owned()]),
        text_files: Some(vec![TextFile {
            src: "files/motd".to_owned(),
            dst: "/etc/motd".to_owned(),
            user: "root".to_owned(),
            group: "root".to_owned(),
            permissions: 644,
        }]),
        binary_files: Some(vec![BinaryFile {
            source: "files/tool.bin".to_owned(),
        }]),
        systemd_units: Some(vec![SystemdUnit {
            name: "lab.service".to_owned(),
            source: "units/lab.service".to_owned(),
            enabled: true,
        }]),
        startup_scripts: Some(vec!["scripts/start.sh".to_owned()]),
        user_scripts: Some(vec!["scripts/user.sh".to_owned()]),
        provisioners: Some(vec![
            Provisioner::Ssh(SshProvisioner {
                files: vec![ProvisionerFile {
                    src: "~/hosts.yml".to_owned(),
                    dst: "~/.config/gh/hosts.yml".to_owned(),
                    mode: 600,
                }],
                env_files: vec!["~/.secrets/dev.env".to_owned()],
                scripts: vec!["./setup.sh".to_owned(), "./more.sh".to_owned()],
            }),
            Provisioner::Container(ContainerProvisioner {
                files: vec![ContainerSecretFile {
                    src: "secrets/db-password".to_owned(),
                    name: "db_password".to_owned(),
                    mode: 444,
                }],
            }),
            Provisioner::Ssh(SshProvisioner::default()),
        ]),
        ..Node::default()
    }
}

/// A manifest that sets every field, with a second node that sets none.
fn full_manifest() -> Manifest {
    Manifest {
        name: "core".to_owned(),
        description: Some("Every field".to_owned()),
        tailscale: Some(TailscaleConfig {
            enabled: true,
            auth_key_env: Some("SHERPA_TAILSCALE_KEY".to_owned()),
        }),
        ready_timeout: Some(900),
        provision_concurrency: Some(3),
        talos_clusters: Some(vec![
            TalosCluster {
                name: "k8s".to_owned(),
                controllers: vec!["cp1".to_owned(), "cp2".to_owned()],
                workers: vec!["w1".to_owned()],
                cni: ClusterCni::Cilium,
                oidc: Some("kc01".to_owned()),
                vault: Some("vault01".to_owned()),
            },
            TalosCluster {
                name: "edge".to_owned(),
                controllers: vec!["cp3".to_owned()],
                ..TalosCluster::default()
            },
        ]),
        nodes: vec![
            full_node(),
            Node {
                name: "web".to_owned(),
                model: NodeModel::Generic,
                kind: Some(NodeKind::Container),
                ..Node::default()
            },
        ],
        links: Some(vec![
            Link2 {
                src: "dev01::eth1".to_owned(),
                dst: "web::eth1".to_owned(),
                p2p: Some(false),
                impairment: Some(ManifestImpairment {
                    delay: Some(20),
                    jitter: Some(2),
                    loss_percent: Some(1.5),
                    reorder_percent: Some(0.25),
                    corrupt_percent: Some(0.1),
                }),
            },
            Link2 {
                src: "dev01::eth2".to_owned(),
                dst: "web::eth2".to_owned(),
                p2p: None,
                impairment: Some(ManifestImpairment::default()),
            },
            Link2 {
                src: "dev01::eth3".to_owned(),
                dst: "web::eth3".to_owned(),
                p2p: None,
                impairment: None,
            },
        ]),
        bridges: Some(vec![
            Bridge {
                name: "br-a".to_owned(),
                links: vec!["dev01::eth4".to_owned(), "web::eth4".to_owned()],
            },
            Bridge {
                name: "br-b".to_owned(),
                links: vec!["dev01::eth5".to_owned()],
            },
        ]),
        ztp_server: Some(ZtpServer {
            enable: true,
            username: Some("ztp".to_owned()),
            password: Some(PASSWORD.to_owned()),
        }),
    }
}

/// Fails to compile when the manifest gains a field, and fails when `manifest` leaves one out:
/// the round trip below then covers every field there is.
fn assert_every_field_set(manifest: &Manifest) {
    let Manifest {
        name,
        description,
        tailscale,
        ready_timeout,
        provision_concurrency,
        talos_clusters,
        nodes,
        links,
        bridges,
        ztp_server,
    } = manifest;
    assert!(!name.is_empty() && description.is_some());
    assert!(tailscale.as_ref().is_some_and(|t| t.auth_key_env.is_some()));
    assert!(ready_timeout.is_some() && provision_concurrency.is_some());
    assert!(talos_clusters.is_some() && links.is_some() && bridges.is_some());
    assert!(
        ztp_server
            .as_ref()
            .is_some_and(|z| z.username.is_some() && z.password.is_some())
    );
    assert_every_node_field_set(&nodes[0]);
}

/// As [`assert_every_field_set`], for a node.
fn assert_every_node_field_set(node: &Node) {
    let Node {
        name: _,
        model: _,
        // The image's kind, held only when it is not the model's default, which `generic` lacks.
        kind: _,
        image,
        version,
        memory,
        cpu,
        numa,
        boot_disk,
        interfaces,
        ztp,
        ssh,
        ready_check,
        vm,
        container,
        unikernel,
        environment_variables,
        text_files,
        binary_files,
        systemd_units,
        startup_scripts,
        startup_scripts_data,
        user_scripts,
        user_scripts_data,
        text_files_data,
        binary_files_data,
        systemd_units_data,
        ssh_authorized_key_files_data,
        provisioners,
    } = node;
    assert!(image.is_some() && version.is_some() && memory.is_some());
    let NodeCpu {
        count,
        mode,
        topology,
        pinning,
        emulator_pinning,
    } = cpu;
    assert!(count.is_some() && mode.is_some() && topology.is_some());
    assert!(pinning.is_some() && emulator_pinning.is_some());
    let NodeNuma {
        node: numa_node,
        memory_mode,
    } = numa;
    assert!(numa_node.is_some() && memory_mode.is_some());
    let NodeBootDisk {
        size,
        clone_method,
        cache,
        io,
        io_thread_cpus,
    } = boot_disk;
    assert!(size.is_some() && clone_method.is_some() && cache.is_some());
    assert!(io.is_some() && io_thread_cpus.is_some());
    let NodeInterfaces { data_count, queues } = interfaces;
    assert!(data_count.is_some() && queues.is_some());
    let NodeZtp { method, config } = ztp;
    assert!(method.is_some() && config.is_some());
    let NodeSsh {
        authorized_keys,
        authorized_key_files,
    } = ssh;
    assert!(authorized_keys.is_some() && authorized_key_files.is_some());
    let NodeReadyCheck { port, skip } = ready_check;
    assert!(port.is_some() && skip.is_some());
    let NodeVm { watchdog } = vm.as_ref().unwrap();
    assert!(watchdog.is_some());
    let NodeContainer {
        user,
        privileged,
        shm_size,
        commands,
        volumes,
        platform,
    } = container.as_ref().unwrap();
    assert!(user.is_some() && privileged.is_some() && shm_size.is_some());
    assert!(commands.is_some() && volumes.is_some() && platform.is_some());
    let NodeUnikernel { cmdline } = unikernel.as_ref().unwrap();
    assert!(cmdline.is_some());
    for list in [
        environment_variables.as_ref().map(Vec::len),
        text_files.as_ref().map(Vec::len),
        binary_files.as_ref().map(Vec::len),
        systemd_units.as_ref().map(Vec::len),
        startup_scripts.as_ref().map(Vec::len),
        user_scripts.as_ref().map(Vec::len),
        provisioners.as_ref().map(Vec::len),
    ] {
        assert!(list.is_some_and(|len| len > 0));
    }
    // The contents the client reads are not part of the manifest as written.
    assert!(startup_scripts_data.is_none() && user_scripts_data.is_none());
    assert!(text_files_data.is_none() && binary_files_data.is_none());
    assert!(systemd_units_data.is_none() && ssh_authorized_key_files_data.is_none());
}

/// A database with a key, and the lab `core` holding the nodes `dev01` (Ubuntu) and `web` (a
/// container), as `up` creates them before it stores the definition.
async fn lab_with_nodes() -> (Database, DbLab) {
    let db = open_in_memory()
        .await
        .unwrap()
        .with_secret_key(SecretKey::generate());
    let lab = lab_in(&db).await;
    (db, lab)
}

/// The lab `core` with its nodes in `db`. Their images have no ZTP password, so a database
/// without a key can hold them.
async fn lab_in(db: &Database) -> DbLab {
    let db = db.clone();
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
            loopback_network: "127.127.1.0/24".to_owned(),
            management_network: "172.31.1.0/24".to_owned(),
            gateway_ipv4: "172.31.1.1".to_owned(),
            router_ipv4: "172.31.1.2".to_owned(),
            management_network_v6: None,
            loopback_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
        },
        &LabDetails {
            description: Some("Every field".to_owned()),
            ..LabDetails::default()
        },
    )
    .await
    .unwrap();
    let lab_id = lab.id.clone().unwrap();
    for (index, (name, model_kind)) in [
        ("dev01", ModelKind::default_for(NodeModel::UbuntuLinux)),
        (
            "web",
            ModelKind::new(NodeModel::Generic, Some(NodeKind::Container)).unwrap(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let image = upsert_node_image(&db, model_kind.node_config(), DefaultRequest::Keep)
            .await
            .unwrap();
        create_node(
            &db,
            name,
            u16::try_from(index + 1).unwrap(),
            image.id.unwrap(),
            lab_id.clone(),
        )
        .await
        .unwrap();
    }
    lab
}

async fn count(db: &Database, sql: &'static str, key: &str) -> usize {
    let conn = db.reader().await.unwrap();
    query_count(&conn, Query::new(sql).bind([key.into()]))
        .await
        .unwrap()
}

/// The rows the lab `key`'s definition has in each child table.
async fn child_rows(db: &Database, key: &str) -> Vec<(&'static str, usize)> {
    let mut rows = Vec::new();
    for table in CHILD_TABLES {
        let sql = if table.starts_with("manifest_node") {
            format!(
                "SELECT count(*) FROM {table} WHERE node IN (SELECT id FROM node WHERE lab = ?1)"
            )
        } else {
            format!("SELECT count(*) FROM {table} WHERE lab = ?1")
        };
        rows.push((table, count(db, sql.leak(), key).await));
    }
    rows
}

fn lab_key(lab: &DbLab) -> String {
    key_of(lab.id.as_ref().unwrap(), LAB_TABLE).unwrap()
}

fn lab_id(lab: &DbLab) -> RecordId {
    lab.id.clone().unwrap()
}

#[tokio::test]
async fn a_manifest_that_sets_every_field_reads_back_equal() {
    let manifest = full_manifest();
    assert_every_field_set(&manifest);
    let (db, lab) = lab_with_nodes().await;

    store_lab_definition(&db, &lab_id(&lab), &manifest)
        .await
        .unwrap();
    let stored = read_lab_definition(&db, &lab_id(&lab)).await.unwrap();
    assert_eq!(stored, Some(manifest));
    for (table, rows) in child_rows(&db, &lab_key(&lab)).await {
        assert!(rows > 0, "{table} has no rows");
    }
}

#[tokio::test]
async fn a_minimal_manifest_reads_back_with_nothing_added() {
    let manifest = Manifest {
        name: "core".to_owned(),
        description: Some("Every field".to_owned()),
        nodes: vec![
            Node {
                name: "dev01".to_owned(),
                model: NodeModel::UbuntuLinux,
                ..Node::default()
            },
            Node {
                name: "web".to_owned(),
                model: NodeModel::Generic,
                kind: Some(NodeKind::Container),
                ..Node::default()
            },
        ],
        ..Manifest::default()
    };
    let (db, lab) = lab_with_nodes().await;
    store_lab_definition(&db, &lab_id(&lab), &manifest)
        .await
        .unwrap();
    assert_eq!(
        read_lab_definition(&db, &lab_id(&lab)).await.unwrap(),
        Some(manifest)
    );
    for (table, rows) in child_rows(&db, &lab_key(&lab)).await {
        assert_eq!(rows, 0, "{table}");
    }
}

#[tokio::test]
async fn empty_lists_and_groups_read_back_as_absent() {
    let mut manifest = full_manifest();
    manifest.links = Some(Vec::new());
    manifest.talos_clusters = Some(Vec::new());
    manifest.nodes[1].environment_variables = Some(Vec::new());
    manifest.nodes[1].vm = Some(NodeVm::default());
    manifest.nodes[1].container = Some(NodeContainer::default());
    let (db, lab) = lab_with_nodes().await;
    store_lab_definition(&db, &lab_id(&lab), &manifest)
        .await
        .unwrap();
    let stored = read_lab_definition(&db, &lab_id(&lab))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.links, None);
    assert_eq!(stored.talos_clusters, None);
    assert_eq!(stored.nodes[1].environment_variables, None);
    assert_eq!(stored.nodes[1].vm, None);
    assert_eq!(stored.nodes[1].container, None);
}

#[tokio::test]
async fn a_lab_without_a_definition_reads_none_until_one_is_stored() {
    let (db, lab) = lab_with_nodes().await;
    assert_eq!(read_lab_definition(&db, &lab_id(&lab)).await.unwrap(), None);

    store_lab_definition(&db, &lab_id(&lab), &full_manifest())
        .await
        .unwrap();
    assert_eq!(
        read_lab_definition(&db, &lab_id(&lab)).await.unwrap(),
        Some(full_manifest())
    );
}

#[tokio::test]
async fn storing_again_replaces_the_whole_definition() {
    let (db, lab) = lab_with_nodes().await;
    store_lab_definition(&db, &lab_id(&lab), &full_manifest())
        .await
        .unwrap();
    let mut smaller = full_manifest();
    smaller.links = None;
    smaller.ztp_server = None;
    smaller.nodes[0] = Node {
        name: "dev01".to_owned(),
        model: NodeModel::UbuntuLinux,
        memory: Some(1024),
        ..Node::default()
    };
    store_lab_definition(&db, &lab_id(&lab), &smaller)
        .await
        .unwrap();
    assert_eq!(
        read_lab_definition(&db, &lab_id(&lab)).await.unwrap(),
        Some(smaller)
    );
}

#[tokio::test]
async fn a_manifest_node_without_a_record_is_refused_and_nothing_is_stored() {
    let (db, lab) = lab_with_nodes().await;
    let mut manifest = full_manifest();
    manifest.nodes.push(Node {
        name: "ghost".to_owned(),
        model: NodeModel::UbuntuLinux,
        ..Node::default()
    });
    let error = store_lab_definition(&db, &lab_id(&lab), &manifest)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("ghost"), "{error:#}");
    assert_eq!(read_lab_definition(&db, &lab_id(&lab)).await.unwrap(), None);
    for (table, rows) in child_rows(&db, &lab_key(&lab)).await {
        assert_eq!(rows, 0, "{table}");
    }
}

#[tokio::test]
async fn the_ztp_server_password_is_stored_sealed() {
    let (db, lab) = lab_with_nodes().await;
    store_lab_definition(&db, &lab_id(&lab), &full_manifest())
        .await
        .unwrap();
    let conn = db.reader().await.unwrap();
    let mut rows = conn
        .query(
            "SELECT ztp_server_password FROM lab WHERE id = ?1",
            [lab_key(&lab)],
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    let Value::Blob(sealed) = row.get_value(0).unwrap() else {
        panic!("the password is not a blob");
    };
    assert!(
        !sealed
            .windows(PASSWORD.len())
            .any(|window| window == PASSWORD.as_bytes())
    );

    let other_key = db.clone().with_secret_key(SecretKey::generate());
    let error = read_lab_definition(&other_key, &lab_id(&lab))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("different database key"),
        "{error:#}"
    );
}

#[tokio::test]
async fn a_password_cannot_be_stored_without_a_key() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir.path().join("sherpa.sqlite")).await.unwrap();
    let lab = lab_in(&db).await;
    let error = store_lab_definition(&db, &lab_id(&lab), &full_manifest())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not loaded"), "{error}");
    let mut manifest = full_manifest();
    manifest.ztp_server = None;
    store_lab_definition(&db, &lab_id(&lab), &manifest)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_redeploy_replaces_its_node_and_the_lab_settings_only() {
    let (db, lab) = lab_with_nodes().await;
    let before = full_manifest();
    store_lab_definition(&db, &lab_id(&lab), &before)
        .await
        .unwrap();

    let mut redeployed = full_manifest();
    redeployed.ready_timeout = Some(60);
    redeployed.provision_concurrency = None;
    redeployed.links = None;
    redeployed.tailscale = None;
    redeployed.nodes[0].memory = Some(8192);
    redeployed.nodes[0].provisioners = None;
    redeployed.nodes[0].environment_variables = Some(vec!["MODE=changed".to_owned()]);
    redeployed.nodes[1].memory = Some(512);
    let node_id = get_node_by_name_and_lab(&db, "dev01", lab_id(&lab))
        .await
        .unwrap()
        .id
        .unwrap();
    store_redeployed_node(&db, &lab_id(&lab), &node_id, "dev01", &redeployed)
        .await
        .unwrap();

    let mut expected = before;
    expected.ready_timeout = Some(60);
    expected.provision_concurrency = None;
    expected.nodes[0] = redeployed.nodes[0].clone();
    assert_eq!(
        read_lab_definition(&db, &lab_id(&lab)).await.unwrap(),
        Some(expected)
    );
}

#[tokio::test]
async fn a_manifest_holding_file_contents_is_refused() {
    let (db, lab) = lab_with_nodes().await;
    let mut resolved = full_manifest();
    resolved.nodes[0].startup_scripts_data = Some(vec![StartupScript {
        filename: "start.sh".to_owned(),
        content: "IyEvYmluL3No".to_owned(),
    }]);
    let error = store_lab_definition(&db, &lab_id(&lab), &resolved)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("contents of files"), "{error}");
    let node_id = get_node_by_name_and_lab(&db, "dev01", lab_id(&lab))
        .await
        .unwrap()
        .id
        .unwrap();
    assert!(
        store_redeployed_node(&db, &lab_id(&lab), &node_id, "dev01", &resolved)
            .await
            .is_err()
    );
    assert_eq!(read_lab_definition(&db, &lab_id(&lab)).await.unwrap(), None);
}

#[tokio::test]
async fn destroying_the_lab_removes_its_whole_definition() {
    let (db, lab) = lab_with_nodes().await;
    store_lab_definition(&db, &lab_id(&lab), &full_manifest())
        .await
        .unwrap();
    assert!(
        delete_lab_from(&db, &lab.lab_id, LabPhase::Creating)
            .await
            .unwrap()
    );
    for table in CHILD_TABLES {
        let sql = format!("SELECT count(*) FROM {table}");
        let conn = db.reader().await.unwrap();
        let rows = query_count(&conn, Query::new(sql.leak()).bind([]))
            .await
            .unwrap();
        assert_eq!(rows, 0, "{table} kept rows of the destroyed lab");
    }
}
