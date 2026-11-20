use shared::data::{
    CpuMode, CpuTopology, DiskCache, DiskImageCloneMethod, DiskIo, NodeKind, NodeModel,
    NumaMemoryMode, PtxPlatform, WatchdogAction, Ztp,
};
use topology::Manifest;
use topology::validate::check_no_ssh_group;

// ============================================================================
// Expected TOML manifests
// ============================================================================

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

[nodes.ready_check]
port = 2222
skip = true

[nodes.vm]
watchdog = "reset"

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

// ============================================================================
// Tests — manifest parsing
// ============================================================================

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

// ============================================================================
// Tests — invalid TOML
// ============================================================================

#[test]
fn test_parse_missing_name() {
    let toml_str = r#"
nodes = [
  { name = "dev01", model = "ubuntu_linux" },
]
"#;
    let error = toml::from_str::<Manifest>(toml_str)
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing field `name`"), "{error}");
}

#[test]
fn test_parse_missing_nodes() {
    let toml_str = r#"
        name = "test-lab"
        "#;
    let error = toml::from_str::<Manifest>(toml_str)
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing field `nodes`"), "{error}");
}

// ============================================================================
// Tests — Manifest::example()
// ============================================================================

// ============================================================================
// Tests — write/load round-trip
// ============================================================================

// ============================================================================
// Tests — Link2::expand()
// ============================================================================

// ============================================================================
// Tests — Bridge::parse_links()
// ============================================================================

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
    let error = toml::from_str::<Manifest>(MANIFEST_WITH_TEXT_FILES_UNKNOWN_FIELD)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown field `unknown_field`"), "{error}");
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

/// The removed `ssh` group, as a table or inline, with keys or key files: each is refused
/// naming the node and pointing to the account's keys.
#[test]
fn test_the_removed_ssh_group_is_refused_pointing_to_the_account_keys() {
    for group in [
        "[nodes.ssh]\nauthorized_keys = ['ssh-ed25519 AAAA']\n",
        "ssh = { authorized_key_files = [{ source = 'keys/admin.pub' }] }\n",
    ] {
        let manifest =
            format!("name = 'lab'\n[[nodes]]\nname = 'dev01'\nmodel = 'ubuntu_linux'\n{group}");
        let manifest = toml::from_str::<Manifest>(&manifest).expect("parses");
        let error = check_no_ssh_group(&manifest.nodes)
            .expect_err("the ssh group is refused")
            .to_string();
        assert!(error.contains("'dev01'"), "{group}: {error}");
        assert!(
            error.contains("`ssh` group, which has been removed"),
            "{group}: {error}"
        );
        assert!(
            error.contains("sherpa server user ssh-key add"),
            "{group}: {error}"
        );
        assert!(error.contains("Profile page"), "{group}: {error}");
    }
}
