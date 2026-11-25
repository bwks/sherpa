use shared::data::{
    ClusterCni, CpuMode, CpuTopology, DiskCache, DiskImageCloneMethod, DiskIo, NodeBootDisk,
    NodeCpu, NodeInterfaces, NodeKind, NodeModel, NodeNuma, NumaMemoryMode, PtxPlatform,
    WatchdogAction, Ztp,
};
use topology::{
    Manifest, ManifestImpairment, Node, NodeExpanded, NodeReadyCheck, NodeZtp, Provisioner,
    SystemdUnit, TailscaleConfig, TextFile,
};

// ============================================================================
// Expected TOML manifests
// ============================================================================

const FULL_MANIFEST: &str = r#"
name = "full-lab"
description = "Every key"
ready_timeout = 600
provision_concurrency = 4

[tailscale]
enabled = true
auth_key_env = "TS_AUTH_KEY"

[[nodes]]
name = "router1"
model = "ubuntu_linux"
version = "24.04"
memory = 2048
environment_variables = ["TERM=xterm", "LANG=en_US.UTF-8"]
startup_scripts = ["scripts/init.sh"]

[nodes.cpu]
count = 2
mode = "host-passthrough"
topology = { sockets = 1, cores = 2, threads = 1 }
pinning = [2, 3]
emulator_pinning = [1]

[nodes.numa]
node = 0
memory_mode = "preferred"

[nodes.boot_disk]
size = 16
clone_method = "copy"
cache = "none"
io = "native"
io_thread_cpus = [1]

[nodes.interfaces]
data_count = 4
queues = 2

[nodes.ztp]
method = "cloud-init"
config = "configs/router1.cfg"

[nodes.ready_check]
port = 2222
skip = true

[nodes.vm]
watchdog = "reset"

[[nodes.text_files]]
src = "/home/user/.config/app.json"
dst = "/etc/app/config.json"
user = "root"
group = "root"
permissions = 644

[[nodes.systemd_units]]
name = "app.service"
source = "units/app.service"
enabled = true

[[nodes]]
name = "server1"
model = "juniper_evolved"
kind = "container"

[nodes.container]
user = "root"
privileged = true
shm_size = 67108864
commands = ["show version", "show ip route"]
volumes = [
  { src = "/data", dst = "/mnt/data" },
  { src = "/logs", dst = "/var/log/app" },
]
platform = "bx"

[[nodes]]
name = "uk1"
model = "nanos_unikernel"

[nodes.unikernel]
cmdline = "console=ttyS0"

[[links]]
src = "router1::eth1"
dst = "server1::eth1"

[[links]]
src = "router1::eth2"
dst = "server1::eth2"
impairment = { delay = 10, jitter = 2, loss_percent = 1.5, reorder_percent = 2.5, corrupt_percent = 0.5 }

[[bridges]]
name = "mgmt-bridge"
links = ["router1::eth3", "server1::eth3"]
"#;

/// A manifest of one Ubuntu node, `dev01`, followed by `rest`.
fn one_node(rest: &str) -> String {
    format!("name = 'lab'\n[[nodes]]\nname = 'dev01'\nmodel = 'ubuntu_linux'\n{rest}\n")
}

#[test]
fn manifest_file_003_a_manifest_without_name_or_nodes_is_refused_at_the_top_level_table() {
    for manifest in [
        "nodes = [{ name = 'dev01', model = 'ubuntu_linux' }]",
        "name = 'lab'",
    ] {
        let error = toml::from_str::<Manifest>(manifest).unwrap_err();
        assert_eq!(error.span(), Some(0..manifest.len()), "{manifest}");
    }
}

/// A table of a node, as TOML that follows the node, and the text the error's span covers: the
/// unknown key it sets, or for a provisioner, which serde reads through its `type` tag, the
/// provisioner's own table.
const UNKNOWN_KEYS_OF_A_NODE: &[(&str, &str, &str)] = &[
    ("a node", "cpu_count = 2", "cpu_count"),
    (
        "cpu.topology",
        "[nodes.cpu]\ntopology = { sockets = 1, cores = 1, threads = 1, unknown = 1 }",
        "unknown",
    ),
    (
        "a volume",
        "[nodes.container]\nvolumes = [{ src = '/a', dst = '/b', unknown = 1 }]",
        "unknown",
    ),
    (
        "a text file",
        "[[nodes.text_files]]\nsrc = 'a'\ndst = '/b'\nuser = 'root'\ngroup = 'root'\npermissions = 644\nunknown = 1",
        "unknown",
    ),
    (
        "a systemd unit",
        "[[nodes.systemd_units]]\nname = 'a.service'\nsource = 'a'\nenabled = true\nunknown = 1",
        "unknown",
    ),
    (
        "an ssh provisioner",
        "[[nodes.provisioners]]\ntype = 'ssh'\ncommands = ['ls']",
        "[[nodes.provisioners]]\ntype = 'ssh'\ncommands = ['ls']",
    ),
    (
        "an ssh provisioner's file",
        "[[nodes.provisioners]]\ntype = 'ssh'\n[[nodes.provisioners.files]]\nsrc = 'a'\ndst = '/b'\nowner = 'root'",
        "[[nodes.provisioners]]\ntype = 'ssh'",
    ),
    (
        "a container provisioner",
        "[[nodes.provisioners]]\ntype = 'container'\nunknown = 1",
        "[[nodes.provisioners]]\ntype = 'container'\nunknown = 1",
    ),
    (
        "a container provisioner's file",
        "[[nodes.provisioners]]\ntype = 'container'\n[[nodes.provisioners.files]]\nsrc = 'a'\nname = 'a'\ndst = '/run/a'",
        "[[nodes.provisioners]]\ntype = 'container'",
    ),
];

/// A table of the lab, as a whole manifest, and the unknown key it sets, which the error's span
/// covers.
const UNKNOWN_KEYS_OF_THE_LAB: &[(&str, &str, &str)] = &[
    (
        "the top level",
        "name = 'lab'\nclusters = []\nnodes = []",
        "clusters",
    ),
    (
        "a link",
        "name = 'lab'\nnodes = []\n[[links]]\nsrc = 'a::eth1'\ndst = 'b::eth1'\nunknown = 1",
        "unknown",
    ),
    (
        "an impairment",
        "name = 'lab'\nnodes = []\n[[links]]\nsrc = 'a::eth1'\ndst = 'b::eth1'\nimpairment = { delay = 1, unknown = 1 }",
        "unknown",
    ),
    (
        "a bridge",
        "name = 'lab'\nnodes = []\n[[bridges]]\nname = 'br'\nlinks = []\nunknown = 1",
        "unknown",
    ),
    (
        "a Talos cluster",
        "name = 'lab'\nnodes = []\n[[talos_clusters]]\nname = 'k8s'\ncontrollers = ['cp1']\nnodes = []",
        "nodes",
    ),
    (
        "[tailscale]",
        "name = 'lab'\nnodes = []\n[tailscale]\nenabled = true\nauth_key = 'tskey-auth-secret'",
        "auth_key",
    ),
];

#[test]
fn manifest_file_004_an_unknown_key_is_refused_in_every_table_at_the_key() {
    let groups = [
        "cpu",
        "numa",
        "boot_disk",
        "interfaces",
        "ztp",
        "ready_check",
        "vm",
        "container",
        "unikernel",
    ]
    .map(|group| {
        (
            group,
            one_node(&format!("[nodes.{group}]\nunknown = 1")),
            "unknown",
        )
    });
    let of_a_node = UNKNOWN_KEYS_OF_A_NODE
        .iter()
        .map(|(table, rest, spanned)| (*table, one_node(rest), *spanned));
    let of_the_lab = UNKNOWN_KEYS_OF_THE_LAB
        .iter()
        .map(|(table, manifest, spanned)| (*table, (*manifest).to_owned(), *spanned));
    for (table, manifest, spanned) in groups.into_iter().chain(of_a_node).chain(of_the_lab) {
        let span = toml::from_str::<Manifest>(&manifest)
            .expect_err(table)
            .span()
            .expect(table);
        assert_eq!(&manifest[span], spanned, "{table}");
    }
}

/// The checks on the virtual machine of `FULL_MANIFEST`, which sets every shared group and
/// `vm`.
fn assert_every_vm_key_is_read(r1: &Node) {
    assert_eq!(r1.name, "router1");
    assert_eq!(r1.model, NodeModel::UbuntuLinux);
    assert_eq!(r1.version, Some("24.04".to_string()));
    assert_eq!(r1.memory, Some(2048));
    assert_eq!(r1.cpu.count, Some(2));
    assert_eq!(r1.cpu.mode, Some(CpuMode::HostPassthrough));
    assert_eq!(
        r1.cpu.topology,
        Some(CpuTopology {
            sockets: 1,
            cores: 2,
            threads: 1
        })
    );
    assert_eq!(r1.cpu.pinning, Some(vec![2, 3]));
    assert_eq!(r1.cpu.emulator_pinning, Some(vec![1]));
    assert_eq!(r1.numa.node, Some(0));
    assert_eq!(r1.numa.memory_mode, Some(NumaMemoryMode::Preferred));
    assert_eq!(r1.boot_disk.size, Some(16));
    assert_eq!(r1.boot_disk.clone_method, Some(DiskImageCloneMethod::Copy));
    assert_eq!(r1.boot_disk.cache, Some(DiskCache::None));
    assert_eq!(r1.boot_disk.io, Some(DiskIo::Native));
    assert_eq!(r1.boot_disk.io_thread_cpus, Some(vec![1]));
    assert_eq!(r1.interfaces.data_count, Some(4));
    assert_eq!(r1.interfaces.queues, Some(2));
    assert_eq!(r1.ztp.method, Some(Ztp::CloudInit));
    assert_eq!(r1.ztp.config, Some("configs/router1.cfg".to_string()));
    assert_eq!(r1.ready_check.port, Some(2222));
    assert_eq!(r1.ready_check.skip, Some(true));
    assert_eq!(
        r1.vm.as_ref().and_then(|vm| vm.watchdog),
        Some(WatchdogAction::Reset)
    );
    assert!(r1.container.is_none());
    assert!(r1.unikernel.is_none());
    assert_eq!(
        r1.environment_variables,
        Some(vec![
            "TERM=xterm".to_string(),
            "LANG=en_US.UTF-8".to_string()
        ])
    );
    assert_eq!(
        r1.startup_scripts,
        Some(vec!["scripts/init.sh".to_string()])
    );
    assert_eq!(
        r1.text_files,
        Some(vec![TextFile {
            src: "/home/user/.config/app.json".to_owned(),
            dst: "/etc/app/config.json".to_owned(),
            user: "root".to_owned(),
            group: "root".to_owned(),
            permissions: 644,
        }])
    );
    assert_eq!(
        r1.systemd_units,
        Some(vec![SystemdUnit {
            name: "app.service".to_owned(),
            source: "units/app.service".to_owned(),
            enabled: true,
        }])
    );
}

#[test]
fn manifest_file_005_every_key_a_manifest_takes_is_read_as_the_setting_it_names() {
    let manifest: Manifest = toml::from_str(FULL_MANIFEST).expect("parses");

    assert_eq!(manifest.name, "full-lab");
    assert_eq!(manifest.description.as_deref(), Some("Every key"));
    assert_eq!(manifest.ready_timeout, Some(600));
    assert_eq!(manifest.provision_concurrency, Some(4));
    assert_eq!(
        manifest.tailscale,
        Some(TailscaleConfig {
            enabled: true,
            auth_key_env: Some("TS_AUTH_KEY".to_owned()),
        })
    );

    assert_every_vm_key_is_read(&manifest.nodes[0]);

    // A container with the container group
    let s1 = &manifest.nodes[1];
    assert_eq!(s1.name, "server1");
    assert_eq!(s1.model, NodeModel::JuniperEvolved);
    assert_eq!(s1.kind, Some(NodeKind::Container));
    let container = s1.container.as_ref().expect("has a container group");
    assert_eq!(container.privileged, Some(true));
    assert_eq!(container.shm_size, Some(67_108_864));
    assert_eq!(container.user, Some("root".to_string()));
    assert_eq!(
        container.commands,
        Some(vec![
            "show version".to_string(),
            "show ip route".to_string()
        ])
    );
    assert_eq!(container.platform, Some(PtxPlatform::Bx));
    let vols = container.volumes.as_ref().expect("has volumes");
    assert_eq!(vols.len(), 2);
    assert_eq!(vols[0].src, "/data");
    assert_eq!(vols[0].dst, "/mnt/data");
    assert_eq!(vols[1].src, "/logs");
    assert_eq!(vols[1].dst, "/var/log/app");

    // A unikernel with the unikernel group
    let u1 = &manifest.nodes[2];
    assert_eq!(
        u1.unikernel
            .as_ref()
            .and_then(|unikernel| unikernel.cmdline.clone()),
        Some("console=ttyS0".to_string())
    );

    let links = manifest.links.as_ref().expect("has links");
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].src, "router1::eth1");
    assert_eq!(links[0].dst, "server1::eth1");
    assert_eq!(links[0].impairment, None);
    assert_eq!(links[1].src, "router1::eth2");
    assert_eq!(links[1].dst, "server1::eth2");
    assert_eq!(
        links[1].impairment,
        Some(ManifestImpairment {
            delay: Some(10),
            jitter: Some(2),
            loss_percent: Some(1.5),
            reorder_percent: Some(2.5),
            corrupt_percent: Some(0.5),
        })
    );

    let bridges = manifest.bridges.as_ref().expect("has bridges");
    assert_eq!(bridges.len(), 1);
    assert_eq!(bridges[0].name, "mgmt-bridge");
    assert_eq!(bridges[0].links, vec!["router1::eth3", "server1::eth3"]);
}

#[test]
fn manifest_file_006_a_key_left_out_stays_unset() {
    let manifest: Manifest = toml::from_str(
        r#"
        name = "lab"

        [[talos_clusters]]
        name = "k8s"
        controllers = ["cp1"]

        [[nodes]]
        name = "dev01"
        model = "ubuntu_linux"

        [[nodes.provisioners]]
        type = "ssh"

        [[nodes.provisioners.files]]
        src = "~/hosts.yml"
        dst = "~/.config/gh/hosts.yml"

        [[nodes]]
        name = "db01"
        model = "mongo_db"

        [[nodes.provisioners]]
        type = "container"

        [[nodes.provisioners.files]]
        src = "secrets/mongo-password"
        name = "mongo_password"
        "#,
    )
    .unwrap();

    let node = &manifest.nodes[0];
    assert_eq!(node.kind, None);
    assert_eq!(node.ztp, NodeZtp::default());
    assert_eq!(NodeExpanded::new(1, node).unwrap().ztp.method, None);
    assert_eq!(node.cpu, NodeCpu::default());
    assert_eq!(node.numa, NodeNuma::default());
    assert_eq!(node.boot_disk, NodeBootDisk::default());
    assert_eq!(node.interfaces, NodeInterfaces::default());
    assert_eq!(node.ready_check, NodeReadyCheck::default());
    assert!(node.vm.is_none() && node.container.is_none() && node.unikernel.is_none());

    assert!(manifest.tailscale.is_none());
    let tailscale: Manifest =
        toml::from_str("name = 'lab'\nnodes = []\n[tailscale]\nauth_key_env = 'TS_AUTH_KEY'")
            .unwrap();
    assert!(!tailscale.tailscale.unwrap().enabled);

    let Some([Provisioner::Ssh(ssh)]) = node.provisioners.as_deref() else {
        panic!("expected one ssh provisioner");
    };
    assert_eq!(ssh.files[0].mode, 600);
    let Some([Provisioner::Container(container)]) = manifest.nodes[1].provisioners.as_deref()
    else {
        panic!("expected one container provisioner");
    };
    assert_eq!(container.files[0].mode, 444);

    let cluster = &manifest.talos_clusters.as_ref().unwrap()[0];
    assert_eq!(cluster.cni, ClusterCni::Flannel);
    assert_eq!(cluster.workers, Vec::<String>::new());
}
