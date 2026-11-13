use shared::data::{
    CpuMode, CpuTopology, DiskCache, DiskImageCloneMethod, DiskIo, NodeModel, NumaMemoryMode,
    PtxPlatform, WatchdogAction, Ztp,
};
use topology::{Bridge, Link2, Manifest, Node};

// ============================================================================
// Expected TOML manifests
// ============================================================================

const MINIMAL_MANIFEST: &str = r#"
name = "test-lab"

nodes = [
  { name = "dev01", model = "ubuntu_linux" },
]
"#;

const FULL_MANIFEST: &str = r#"
name = "full-lab"
ready_timeout = 600

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

[nodes.ssh]
authorized_keys = ["ssh-rsa AAAA... user@host"]
authorized_key_files = [{ source = "keys/admin.pub" }]

[nodes.ready_check]
port = 2222
skip = true

[nodes.vm]
watchdog = "reset"

[[nodes]]
name = "server1"
model = "juniper_cevolved"

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

[[bridges]]
name = "mgmt-bridge"
links = ["router1::eth3", "server1::eth3"]

[ztp_server]
enable = true
username = "admin"
password = "secret123"
"#;

const MANIFEST_WITH_TEXT_FILES: &str = r#"
    name = "text-files-lab"

    [[nodes]]
    name = "dev01"
    model = "ubuntu_linux"

    [[nodes.text_files]]
    src = "/home/user/.config/app.json"
    dst = "/etc/app/config.json"
    user = "root"
    group = "root"
    permissions = 644

    [[nodes.text_files]]
    src = "relative/path.txt"
    dst = "/opt/data/path.txt"
    user = "sherpa"
    group = "sherpa"
    permissions = 755
    "#;

const MANIFEST_WITH_TEXT_FILES_UNKNOWN_FIELD: &str = r#"
    name = "bad-lab"

    [[nodes]]
    name = "dev01"
    model = "ubuntu_linux"

    [[nodes.text_files]]
    src = "/tmp/file.txt"
    dst = "/etc/file.txt"
    user = "root"
    group = "root"
    permissions = 644
    unknown_field = "bad"
    "#;

const MANIFEST_WITH_VOLUMES: &str = r#"
name = "vol-lab"

[[nodes]]
name = "app1"
model = "generic_container"

[nodes.container]
volumes = [
  { src = "/host/data", dst = "/container/data" },
  { src = "/host/config", dst = "/etc/app" },
]
"#;

// ============================================================================
// Tests — manifest parsing
// ============================================================================

#[test]
fn test_parse_minimal_manifest() {
    let manifest: Manifest = toml::from_str(MINIMAL_MANIFEST).expect("parses");
    assert_eq!(manifest.name, "test-lab");
    assert_eq!(manifest.nodes.len(), 1);
    assert_eq!(manifest.nodes[0].name, "dev01");
    assert_eq!(manifest.nodes[0].model, NodeModel::UbuntuLinux);
    assert!(manifest.links.is_none());
    assert!(manifest.bridges.is_none());
    assert!(manifest.ready_timeout.is_none());
    assert!(manifest.ztp_server.is_none());
}

#[test]
fn test_parse_full_manifest() {
    let manifest: Manifest = toml::from_str(FULL_MANIFEST).expect("parses");

    // Top-level fields
    assert_eq!(manifest.name, "full-lab");
    assert_eq!(manifest.ready_timeout, Some(600));

    // Node 1 — a VM with every shared group and the vm group
    let r1 = &manifest.nodes[0];
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
    let keys = r1.ssh.authorized_keys.as_ref().expect("has keys");
    assert_eq!(keys[0], "ssh-rsa AAAA... user@host");
    let key_files = r1.ssh.authorized_key_files.as_ref().expect("has key files");
    assert_eq!(key_files[0].source, "keys/admin.pub");
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

    // Node 2 — a container with the container group
    let s1 = &manifest.nodes[1];
    assert_eq!(s1.name, "server1");
    assert_eq!(s1.model, NodeModel::JuniperCevolved);
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

    // Node 3 — a unikernel with the unikernel group
    let u1 = &manifest.nodes[2];
    assert_eq!(
        u1.unikernel
            .as_ref()
            .and_then(|unikernel| unikernel.cmdline.clone()),
        Some("console=ttyS0".to_string())
    );

    // Links
    let links = manifest.links.as_ref().expect("has links");
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].src, "router1::eth1");
    assert_eq!(links[0].dst, "server1::eth1");
    assert_eq!(links[0].p2p, None);
    assert_eq!(links[1].src, "router1::eth2");
    assert_eq!(links[1].dst, "server1::eth2");
    assert_eq!(links[1].p2p, None);

    // Bridges
    let bridges = manifest.bridges.as_ref().expect("has bridges");
    assert_eq!(bridges.len(), 1);
    assert_eq!(bridges[0].name, "mgmt-bridge");
    assert_eq!(bridges[0].links, vec!["router1::eth3", "server1::eth3"]);

    // ZTP server
    let ztp = manifest.ztp_server.as_ref().expect("has ztp_server");
    assert!(ztp.enable);
    assert_eq!(ztp.username, Some("admin".to_string()));
    assert_eq!(ztp.password, Some("secret123".to_string()));
}

#[test]
fn test_parse_volumes() {
    let manifest: Manifest = toml::from_str(MANIFEST_WITH_VOLUMES).expect("parses");
    let vols = manifest.nodes[0]
        .container
        .as_ref()
        .and_then(|container| container.volumes.as_ref())
        .expect("has volumes");
    assert_eq!(vols.len(), 2);
    assert_eq!(vols[0].src, "/host/data");
    assert_eq!(vols[0].dst, "/container/data");
    assert_eq!(vols[1].src, "/host/config");
    assert_eq!(vols[1].dst, "/etc/app");
}

// ============================================================================
// Tests — invalid TOML
// ============================================================================

#[test]
fn test_parse_invalid_toml() {
    let result = toml::from_str::<Manifest>("this is not valid toml {{{}}}");
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_name() {
    let toml_str = r#"
nodes = [
  { name = "dev01", model = "ubuntu_linux" },
]
"#;
    let result = toml::from_str::<Manifest>(toml_str);
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_nodes() {
    let toml_str = r#"
        name = "test-lab"
        "#;
    let result = toml::from_str::<Manifest>(toml_str);
    assert!(result.is_err());
}

#[test]
fn test_parse_node_rejects_unknown_data_interface_field() {
    let toml_str = r#"
name = "test-lab"

nodes = [
  { name = "dev01", model = "ubuntu_linux", data_interfaces = 4 },
]
"#;
    let result = toml::from_str::<Manifest>(toml_str);
    assert!(result.is_err());
}

#[test]
fn test_parse_node_rejects_management_addresses() {
    for field in [
        r#"ipv4_address = "10.0.0.10""#,
        r#"ipv6_address = "fd00::10""#,
    ] {
        let toml_str = format!(
            r#"
            name = "test-lab"

            [[nodes]]
            name = "dev01"
            model = "ubuntu_linux"
            {field}
            "#
        );
        let error = toml::from_str::<Manifest>(&toml_str).unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");
    }
}

// ============================================================================
// Tests — Manifest::example()
// ============================================================================

#[test]
fn test_example_manifest() {
    let manifest = Manifest::example().expect("generates example");
    assert_ne!(manifest.name, "");
    assert_eq!(manifest.nodes.len(), 2);
    assert_eq!(manifest.nodes[0].name, "dev01");
    assert_eq!(manifest.nodes[0].model, NodeModel::UbuntuLinux);
    assert_eq!(manifest.nodes[1].name, "dev02");
    assert_eq!(manifest.nodes[1].model, NodeModel::FedoraLinux);

    let links = manifest.links.as_ref().expect("has links");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].src, "dev01::eth1");
    assert_eq!(links[0].dst, "dev02::eth1");
}

// ============================================================================
// Tests — write/load round-trip
// ============================================================================

#[test]
fn test_write_load_roundtrip() {
    let manifest = Manifest {
        provision_current_user: true,
        tailscale: None,
        talos_clusters: None,
        name: "roundtrip-lab".to_string(),
        description: Some("Practice routing between two nodes.".into()),
        ready_timeout: None,
        provision_concurrency: None,
        nodes: vec![
            Node {
                name: "r1".to_string(),
                model: NodeModel::CiscoIosv,
                ..Default::default()
            },
            Node {
                name: "r2".to_string(),
                model: NodeModel::AristaVeos,
                ..Default::default()
            },
        ],
        links: Some(vec![Link2 {
            src: "r1::eth1".to_string(),
            dst: "r2::eth1".to_string(),
            p2p: None,
            impairment: None,
        }]),
        bridges: None,
        ztp_server: None,
    };

    let tmp_path = "/tmp/sherpa_test_manifest_roundtrip.toml";
    manifest.write_file(tmp_path).expect("writes file");
    let loaded = Manifest::load_file(tmp_path).expect("loads file");

    assert_eq!(loaded.name, "roundtrip-lab");
    assert_eq!(loaded.description, manifest.description);
    assert!(loaded.provision_current_user);
    assert_eq!(loaded.nodes.len(), 2);
    assert_eq!(loaded.nodes[0].name, "r1");
    assert_eq!(loaded.nodes[0].model, NodeModel::CiscoIosv);
    assert_eq!(loaded.nodes[1].name, "r2");
    assert_eq!(loaded.nodes[1].model, NodeModel::AristaVeos);

    let links = loaded.links.as_ref().expect("has links");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].src, "r1::eth1");
    assert_eq!(links[0].dst, "r2::eth1");

    std::fs::remove_file(tmp_path).ok();
}

#[test]
fn test_write_file_keeps_every_field() {
    let input = format!(
        r#"
        {FULL_MANIFEST}

        [[talos_clusters]]
        name = "k8s"
        controllers = ["router1"]

        [tailscale]
        enabled = true
        auth_key_env = "TS_KEY"
        "#
    );
    let manifest: Manifest = toml::from_str(&input).expect("parses full manifest");

    let tmp_path = std::env::temp_dir().join("sherpa_test_manifest_every_field.toml");
    let tmp_path = tmp_path.to_str().expect("utf-8 temp path");
    manifest.write_file(tmp_path).expect("writes file");
    let loaded = Manifest::load_file(tmp_path).expect("loads file");
    std::fs::remove_file(tmp_path).ok();

    assert_eq!(
        toml::Value::try_from(&loaded).unwrap(),
        toml::Value::try_from(&manifest).unwrap()
    );
}

#[test]
fn test_write_file_emits_tables_without_resolved_data() {
    let manifest = Manifest::example().expect("builds example");

    let tmp_path = std::env::temp_dir().join("sherpa_test_manifest_example.toml");
    let tmp_path = tmp_path.to_str().expect("utf-8 temp path");
    manifest.write_file(tmp_path).expect("writes file");
    let contents = std::fs::read_to_string(tmp_path).expect("reads file");
    std::fs::remove_file(tmp_path).ok();

    assert!(contents.contains("[[nodes]]"));
    assert!(contents.contains("[[links]]"));
    assert!(!contents.contains("_data"));
}

// ============================================================================
// Tests — Link2::expand()
// ============================================================================

#[test]
fn test_link2_expand() {
    let link = Link2 {
        src: "router1::eth1".to_string(),
        dst: "switch1::GigabitEthernet0/1".to_string(),
        p2p: None,
        impairment: None,
    };
    let expanded = link.expand().expect("expands");
    assert_eq!(expanded.node_a, "router1");
    assert_eq!(expanded.int_a, "eth1");
    assert_eq!(expanded.node_b, "switch1");
    assert_eq!(expanded.int_b, "GigabitEthernet0/1");
}

#[test]
fn test_link2_expand_missing_separator() {
    let link = Link2 {
        src: "router1-eth1".to_string(),
        dst: "switch1::eth1".to_string(),
        p2p: None,
        impairment: None,
    };
    let result = link.expand();
    assert!(result.is_err());
}

// ============================================================================
// Tests — Bridge::parse_links()
// ============================================================================

#[test]
fn test_bridge_parse_links() {
    let bridge = Bridge {
        name: "br0".to_string(),
        links: vec![
            "router1::eth3".to_string(),
            "switch1::eth5".to_string(),
            "server1::eth2".to_string(),
        ],
    };
    let expanded = bridge.parse_links().expect("parses");
    assert_eq!(expanded.name, "br0");
    assert_eq!(expanded.links.len(), 3);
    assert_eq!(expanded.links[0].node, "router1");
    assert_eq!(expanded.links[0].interface, "eth3");
    assert_eq!(expanded.links[1].node, "switch1");
    assert_eq!(expanded.links[1].interface, "eth5");
    assert_eq!(expanded.links[2].node, "server1");
    assert_eq!(expanded.links[2].interface, "eth2");
}

#[test]
fn test_bridge_parse_links_invalid_format() {
    let bridge = Bridge {
        name: "br0".to_string(),
        links: vec![
            "router1::eth3".to_string(),
            "bad-format-no-separator".to_string(),
        ],
    };
    let result = bridge.parse_links();
    assert!(result.is_err());
}

// ============================================================================
// Tests — Node defaults
// ============================================================================

#[test]
fn test_node_defaults() {
    let node = Node {
        name: "test".to_string(),
        model: NodeModel::UbuntuLinux,
        ..Default::default()
    };
    assert!(node.version.is_none());
    assert!(node.cpu.count.is_none());
    assert!(node.memory.is_none());
    assert!(node.boot_disk.size.is_none());
    assert!(node.interfaces.data_count.is_none());
    assert!(node.container.is_none());
    assert!(node.vm.is_none());
    assert!(node.unikernel.is_none());
    assert!(node.environment_variables.is_none());
    assert!(node.ready_check.skip.is_none());
    assert!(node.ztp.config.is_none());
    assert!(node.startup_scripts.is_none());
    assert!(node.text_files.is_none());
}

// ============================================================================
// Tests — text_files parsing
// ============================================================================

#[test]
fn test_parse_text_files() {
    let manifest: Manifest = toml::from_str(MANIFEST_WITH_TEXT_FILES).expect("parses");
    let node = &manifest.nodes[0];
    let text_files = node.text_files.as_ref().expect("has text_files");
    assert_eq!(text_files.len(), 2);

    assert_eq!(text_files[0].src, "/home/user/.config/app.json");
    assert_eq!(text_files[0].dst, "/etc/app/config.json");
    assert_eq!(text_files[0].user, "root");
    assert_eq!(text_files[0].group, "root");
    assert_eq!(text_files[0].permissions, 644);

    assert_eq!(text_files[1].src, "relative/path.txt");
    assert_eq!(text_files[1].dst, "/opt/data/path.txt");
    assert_eq!(text_files[1].user, "sherpa");
    assert_eq!(text_files[1].group, "sherpa");
    assert_eq!(text_files[1].permissions, 755);
}

#[test]
fn test_parse_text_files_rejects_unknown_fields() {
    let result = toml::from_str::<Manifest>(MANIFEST_WITH_TEXT_FILES_UNKNOWN_FIELD);
    assert!(result.is_err());
}

#[test]
fn test_config_management_section_rejected() {
    let toml = format!("{MINIMAL_MANIFEST}\n[config_management]\nansible = true\n");
    assert!(toml::from_str::<Manifest>(&toml).is_err());
}

// ============================================================================
// Tests — keys of earlier manifests are refused
// ============================================================================

/// Each flat key of earlier manifests, and a group key under its old name, with a value of
/// its type. Each is refused with serde's unknown-field error naming it.
#[test]
fn test_old_flat_keys_are_refused() {
    for (key, value) in [
        ("cpu_count", "2"),
        ("boot_disk_size", "20"),
        ("boot_disk_clone_method", "'copy'"),
        ("data_interface_count", "2"),
        ("ssh_authorized_keys", "['ssh-ed25519 AAAA']"),
        ("ssh_authorized_key_files", "[{ source = 'a.pub' }]"),
        ("commands", "['ls']"),
        ("volumes", "[{ src = '/a', dst = '/b' }]"),
        ("privileged", "true"),
        ("shm_size", "1024"),
        ("user", "'root'"),
        ("skip_ready_check", "true"),
        ("ready_port", "22"),
        ("ztp_config", "'a.cfg'"),
        ("kernel_cmdline", "'console=ttyS0'"),
        ("watchdog", "'reset'"),
        ("platform", "'bt'"),
        ("performance_tuning", "{ cpu_mode = 'host-model' }"),
    ] {
        let manifest = format!(
            "name = 'lab'\n[[nodes]]\nname = 'dev01'\nmodel = 'ubuntu_linux'\n{key} = {value}\n"
        );
        let error = toml::from_str::<Manifest>(&manifest)
            .expect_err("an old key is refused")
            .to_string();
        assert!(
            error.contains(&format!("unknown field `{key}`")),
            "{key}: {error}"
        );
    }
}

#[test]
fn test_old_keys_inside_a_group_are_refused() {
    let manifest = "name = 'lab'\n[[nodes]]\nname = 'dev01'\nmodel = 'ubuntu_linux'\n[nodes.cpu]\ncpu_pinning = [1]\n";
    let error = toml::from_str::<Manifest>(manifest)
        .expect_err("an old key in a group is refused")
        .to_string();
    assert!(error.contains("unknown field `cpu_pinning`"), "{error}");
}

#[test]
fn test_ztp_as_a_bare_word_is_refused() {
    let manifest =
        "name = 'lab'\n[[nodes]]\nname = 'dev01'\nmodel = 'ubuntu_linux'\nztp = 'cloud-init'\n";
    assert!(toml::from_str::<Manifest>(manifest).is_err());
}
