use serde_json::Value;

use shared::data::ClusterRole;
use template::{
    CiliumConfig, ClusterOidcConfig, ClusterVaultConfig, TalosClientCredentials,
    TalosClusterSecrets, TalosNodeConfig, kubeconfig, talos_machine_config, talosconfig,
};

/// The documents of `talosctl gen config` for Talos 1.14, without the install and discovery
/// documents.
const LAB_CA: &str = "-----BEGIN CERTIFICATE-----\nlab\n-----END CERTIFICATE-----\n";

const CONTROL_PLANE_KINDS: [&str; 25] = [
    "v1alpha1",
    "ResolverConfig",
    "VolumeConfig",
    "FilesystemTrimConfig",
    "SecurityProfileConfig",
    "HostnameConfig",
    "KubeClusterConfig",
    "KubeNodeConfig",
    "KubeletConfig",
    "KubeNetworkConfig",
    "KubeAPIServerCAConfig",
    "KubePrismConfig",
    "KubeAggregatorCAConfig",
    "KubeAdmissionControlConfig",
    "KubeAuditPolicyConfig",
    "KubeAuthenticationConfig",
    "KubeAuthorizerConfig",
    "KubeAuthorizerConfig",
    "KubeEtcdEncryptionConfig",
    "KubeServiceAccountConfig",
    "KubeAPIServerConfig",
    "KubeControllerManagerConfig",
    "KubeSchedulerConfig",
    "KubeProxyConfig",
    "KubeFlannelCNIConfig",
];

fn secrets() -> TalosClusterSecrets {
    TalosClusterSecrets {
        os_ca_cert: "-----BEGIN CERTIFICATE-----\nos\n-----END CERTIFICATE-----\n".to_owned(),
        os_ca_key: "-----BEGIN PRIVATE KEY-----\nos\n-----END PRIVATE KEY-----\n".to_owned(),
        etcd_ca_cert: "etcd-cert".to_owned(),
        etcd_ca_key: "etcd-key".to_owned(),
        kubernetes_ca_cert: "k8s-cert".to_owned(),
        kubernetes_ca_key: "k8s-key".to_owned(),
        aggregator_ca_cert: "agg-cert".to_owned(),
        aggregator_ca_key: "agg-key".to_owned(),
        service_account_key: "sa-key".to_owned(),
        bootstrap_token: "abcdef.0123456789abcdef".to_owned(),
        trustd_token: "ghijkl.0123456789abcdef".to_owned(),
        secretbox_encryption_secret: "c2VjcmV0".to_owned(),
    }
}

fn render(
    role: ClusterRole,
    vip: Option<&str>,
    schedulable: bool,
    cilium: Option<CiliumConfig>,
) -> Vec<Value> {
    let secrets = secrets();
    let node = TalosNodeConfig {
        cluster_name: "k8s",
        endpoint: "https://172.31.1.5:6443",
        hostname: "cp1",
        role,
        management_subnet: "172.31.1.0/24",
        vip: vip.map(|ip| ip.parse().unwrap()),
        schedulable_control_planes: schedulable,
        cilium,
        oidc: None,
        vault: None,
        secrets: &secrets,
    };
    render_node(&node)
}

/// A control plane of a cluster that logs in through Keycloak and has a Vault.
fn render_with_keycloak_and_vault() -> Vec<Value> {
    let secrets = secrets();
    render_node(&TalosNodeConfig {
        cluster_name: "k8s",
        endpoint: "https://172.31.1.5:6443",
        hostname: "cp1",
        role: ClusterRole::ControlPlane,
        management_subnet: "172.31.1.0/24",
        vip: None,
        schedulable_control_planes: false,
        cilium: None,
        oidc: Some(oidc()),
        vault: Some(ClusterVaultConfig {
            address: "https://172.31.1.24:8200".to_owned(),
            ca_cert: LAB_CA,
            auth_mount: "kubernetes-k8s".to_owned(),
        }),
        secrets: &secrets,
    })
}

fn oidc() -> ClusterOidcConfig<'static> {
    ClusterOidcConfig {
        issuer_url: "https://172.31.1.23:8443/realms/sherpa".to_owned(),
        ca_cert: LAB_CA,
    }
}

fn render_node(node: &TalosNodeConfig) -> Vec<Value> {
    let output = talos_machine_config(node).unwrap();
    assert!(output.ends_with('\n'));
    output
        .trim_end()
        .split("\n---\n")
        .map(|document| serde_json::from_str(document).unwrap())
        .collect()
}

fn kinds(documents: &[Value]) -> Vec<String> {
    documents
        .iter()
        .map(|document| document["kind"].as_str().unwrap_or("v1alpha1").to_owned())
        .collect()
}

fn cilium() -> CiliumConfig {
    CiliumConfig {
        load_balancer_first: "172.31.1.13".parse().unwrap(),
        load_balancer_last: "172.31.1.16".parse().unwrap(),
        gateway: "172.31.1.13".parse().unwrap(),
        hubble_ui: "172.31.1.14".parse().unwrap(),
        operator_replicas: 2,
    }
}

/// The resources of the inline manifest `name`, parsed from its `---`-separated JSON stream.
fn inline_manifest(documents: &[Value], name: &str) -> Vec<Value> {
    let manifests = documents[0]["cluster"]["inlineManifests"]
        .as_array()
        .unwrap();
    let manifest = manifests
        .iter()
        .find(|manifest| manifest["name"] == name)
        .unwrap();
    manifest["contents"]
        .as_str()
        .unwrap()
        .split("\n---\n")
        .map(|resource| serde_json::from_str(resource).unwrap())
        .collect()
}

fn find<'a>(documents: &'a [Value], kind: &str) -> &'a Value {
    documents
        .iter()
        .find(|document| document["kind"] == kind)
        .unwrap()
}

#[test]
fn control_plane_config_matches_the_talos_document_set() {
    let documents = render(ClusterRole::ControlPlane, None, false, None);
    let mut expected: Vec<String> = CONTROL_PLANE_KINDS
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    expected.push("KubeCoreDNSConfig".to_owned());
    assert_eq!(kinds(&documents), expected);
    let legacy = &documents[0];
    assert_eq!(legacy["machine"]["type"], "controlplane");
    assert_eq!(legacy["machine"]["token"], "ghijkl.0123456789abcdef");
    assert_eq!(legacy["cluster"]["token"], "abcdef.0123456789abcdef");
    // Legacy documents carry PEM as base64; typed documents carry raw PEM.
    let os_ca =
        shared::util::base64_decode(legacy["machine"]["ca"]["crt"].as_str().unwrap()).unwrap();
    assert!(os_ca.starts_with("-----BEGIN CERTIFICATE-----"));
    assert_eq!(
        find(&documents, "KubeAPIServerCAConfig")["issuingCA"]["key"],
        "k8s-key"
    );
    assert_eq!(
        find(&documents, "KubeClusterConfig")["endpoint"],
        "https://172.31.1.5:6443"
    );
    assert_eq!(find(&documents, "KubeClusterConfig")["clusterName"], "k8s");
    assert_eq!(find(&documents, "HostnameConfig")["hostname"], "cp1");
    assert_eq!(
        find(&documents, "KubeServiceAccountConfig")["issuer"]["issuerURL"],
        "https://172.31.1.5:6443"
    );
    assert_eq!(
        find(&documents, "KubeNodeConfig")["nodeIP"]["validSubnets"][0],
        "172.31.1.0/24"
    );
    assert_eq!(
        find(&documents, "KubeletConfig")["image"],
        "ghcr.io/siderolabs/kubelet:v1.37.1"
    );
    assert!(
        !kinds(&documents)
            .iter()
            .any(|kind| kind == "UnattendedInstallConfig"
                || kind.starts_with("Discovery")
                || kind == "Layer2VIPConfig")
    );
}

#[test]
fn worker_config_carries_no_cluster_private_keys() {
    let documents = render(ClusterRole::Worker, None, false, None);
    assert_eq!(
        kinds(&documents),
        [
            "v1alpha1",
            "ResolverConfig",
            "VolumeConfig",
            "FilesystemTrimConfig",
            "SecurityProfileConfig",
            "HostnameConfig",
            "KubeClusterConfig",
            "KubeNodeConfig",
            "KubeletConfig",
            "KubeNetworkConfig",
            "KubeAPIServerCAConfig",
            "KubePrismConfig",
        ]
    );
    let legacy = &documents[0];
    assert_eq!(legacy["machine"]["type"], "worker");
    assert_eq!(legacy["machine"]["ca"]["key"], "");
    assert!(legacy["cluster"].get("etcd").is_none());
    assert_eq!(
        find(&documents, "KubeAPIServerCAConfig")["acceptedCAs"][0],
        "k8s-cert"
    );
    let output = documents.iter().map(Value::to_string).collect::<String>();
    for secret in [
        "os\\n-----END PRIVATE KEY",
        "etcd-key",
        "k8s-key",
        "agg-key",
        "sa-key",
        "c2VjcmV0",
    ] {
        assert!(!output.contains(secret), "worker config leaks {secret}");
    }
}

#[test]
fn control_planes_share_a_vip_and_drop_the_taint_when_there_are_no_workers() {
    let documents = render(ClusterRole::ControlPlane, Some("172.31.1.5"), true, None);
    let vip = find(&documents, "Layer2VIPConfig");
    assert_eq!(vip["name"], "172.31.1.5");
    assert_eq!(vip["link"], "eth0");
    assert!(find(&documents, "KubeNodeConfig").get("taints").is_none());
    let tainted = render(ClusterRole::ControlPlane, None, false, None);
    assert_eq!(
        find(&tainted, "KubeNodeConfig")["taints"]["node-role.kubernetes.io/control-plane"],
        "NoSchedule"
    );
}

fn credentials() -> TalosClientCredentials<'static> {
    TalosClientCredentials {
        ca_cert: "ca-pem",
        client_cert: "cert-pem",
        client_key: "key-pem",
    }
}

#[test]
fn talosconfig_targets_the_control_planes() {
    let output = talosconfig(
        "k8s",
        &["172.31.1.10".to_owned(), "172.31.1.11".to_owned()],
        &["172.31.1.10".to_owned()],
        &credentials(),
    )
    .unwrap();
    let document: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(document["context"], "k8s");
    let context = &document["contexts"]["k8s"];
    assert_eq!(context["endpoints"][1], "172.31.1.11");
    assert_eq!(context["nodes"][0], "172.31.1.10");
    assert_eq!(
        shared::util::base64_decode(context["crt"].as_str().unwrap()).unwrap(),
        "cert-pem"
    );
    assert_eq!(
        shared::util::base64_decode(context["key"].as_str().unwrap()).unwrap(),
        "key-pem"
    );
}

#[test]
fn kubeconfig_uses_the_cluster_endpoint_and_admin_credentials() {
    let output = kubeconfig("k8s", "https://172.31.1.5:6443", &credentials(), None).unwrap();
    let document: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(document["current-context"], "admin@k8s");
    assert_eq!(
        document["clusters"][0]["cluster"]["server"],
        "https://172.31.1.5:6443"
    );
    let user = &document["users"][0]["user"];
    assert_eq!(
        shared::util::base64_decode(user["client-certificate-data"].as_str().unwrap()).unwrap(),
        "cert-pem"
    );
}

#[test]
fn cilium_control_planes_drop_flannel_and_kube_proxy() {
    let documents = render(ClusterRole::ControlPlane, None, false, Some(cilium()));
    let mut expected: Vec<String> = CONTROL_PLANE_KINDS
        .iter()
        .filter(|kind| **kind != "KubeFlannelCNIConfig")
        .map(std::string::ToString::to_string)
        .collect();
    expected.push("KubeCoreDNSConfig".to_owned());
    assert_eq!(kinds(&documents), expected);
    assert_eq!(find(&documents, "KubeProxyConfig")["enabled"], false);
    assert_eq!(
        documents[0]["cluster"]["extraManifests"][0],
        "https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.6.3/standard-install.yaml"
    );
    let names: Vec<&str> = documents[0]["cluster"]["inlineManifests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|manifest| manifest["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["cilium-install", "cilium-load-balancer", "cilium-gateway"]
    );
}

#[test]
fn flannel_control_planes_keep_kube_proxy_and_carry_no_manifests() {
    let documents = render(ClusterRole::ControlPlane, None, false, None);
    assert!(find(&documents, "KubeProxyConfig").get("enabled").is_none());
    assert!(documents[0]["cluster"].get("inlineManifests").is_none());
    assert!(documents[0]["cluster"].get("extraManifests").is_none());
}

#[test]
fn cilium_workers_are_unchanged() {
    assert_eq!(
        render(ClusterRole::Worker, None, false, Some(cilium())),
        render(ClusterRole::Worker, None, false, None)
    );
}

#[test]
fn cilium_is_installed_by_a_job_on_a_control_plane_host_network() {
    let documents = render(ClusterRole::ControlPlane, None, false, Some(cilium()));
    let resources = inline_manifest(&documents, "cilium-install");
    let kinds: Vec<&str> = resources
        .iter()
        .map(|resource| resource["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["ServiceAccount", "ClusterRoleBinding", "Job"]);
    let pod = &resources[2]["spec"]["template"]["spec"];
    assert_eq!(pod["hostNetwork"], true);
    assert_eq!(pod["dnsPolicy"], "Default");
    assert_eq!(pod["tolerations"][0]["operator"], "Exists");
    let container = &pod["containers"][0];
    assert_eq!(container["image"], "quay.io/cilium/cilium-cli:v0.20.2");
    assert_eq!(container["env"][1]["value"], "7445");
    let command: Vec<&str> = container["command"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    for arg in [
        "install",
        "--version=1.20.4",
        "--set=kubeProxyReplacement=true",
        "--set=k8sServiceHost=localhost",
        "--set=k8sServicePort=7445",
        "--set=bpf.hostLegacyRouting=true",
        "--set=gatewayAPI.enabled=true",
        "--set=l2announcements.enabled=true",
        "--set=hubble.ui.enabled=true",
        "--set=operator.replicas=2",
    ] {
        assert!(command.contains(&arg), "missing {arg} in {command:?}");
    }
}

#[test]
fn cilium_announces_its_load_balancer_block_on_the_management_link() {
    let documents = render(ClusterRole::ControlPlane, None, false, Some(cilium()));
    let resources = inline_manifest(&documents, "cilium-load-balancer");
    assert_eq!(resources[0]["kind"], "CiliumLoadBalancerIPPool");
    assert_eq!(resources[0]["spec"]["blocks"][0]["start"], "172.31.1.13");
    assert_eq!(resources[0]["spec"]["blocks"][0]["stop"], "172.31.1.16");
    assert_eq!(resources[1]["kind"], "CiliumL2AnnouncementPolicy");
    assert_eq!(resources[1]["spec"]["loadBalancerIPs"], true);
    assert_eq!(resources[1]["spec"]["interfaces"][0], "^eth0$");
}

#[test]
fn cilium_clusters_get_a_gateway_and_a_hubble_ui_address() {
    let documents = render(ClusterRole::ControlPlane, None, false, Some(cilium()));
    let resources = inline_manifest(&documents, "cilium-gateway");
    let gateway = &resources[0];
    assert_eq!(gateway["kind"], "Gateway");
    assert_eq!(gateway["spec"]["gatewayClassName"], "cilium");
    assert_eq!(gateway["spec"]["addresses"][0]["value"], "172.31.1.13");
    assert_eq!(gateway["spec"]["listeners"][0]["port"], 80);
    let hubble = &resources[1];
    assert_eq!(hubble["spec"]["type"], "LoadBalancer");
    assert_eq!(
        hubble["metadata"]["annotations"]["lbipam.cilium.io/ips"],
        "172.31.1.14"
    );
    assert_eq!(hubble["spec"]["selector"]["k8s-app"], "hubble-ui");
}

#[test]
fn clusters_without_keycloak_have_no_jwt_authenticator() {
    let documents = render(ClusterRole::ControlPlane, None, false, None);
    assert_eq!(
        find(&documents, "KubeAuthenticationConfig")["configuration"]["jwt"],
        serde_json::json!([])
    );
}

#[test]
fn keycloak_clusters_accept_realm_tokens_with_prefixed_names() {
    let documents = render_with_keycloak_and_vault();
    let jwt = &find(&documents, "KubeAuthenticationConfig")["configuration"]["jwt"][0];
    assert_eq!(
        jwt["issuer"]["url"],
        "https://172.31.1.23:8443/realms/sherpa"
    );
    assert_eq!(
        jwt["issuer"]["audiences"],
        serde_json::json!(["kubernetes"])
    );
    assert_eq!(jwt["issuer"]["certificateAuthority"], LAB_CA);
    assert_eq!(
        jwt["claimMappings"]["username"]["claim"],
        "preferred_username"
    );
    assert_eq!(jwt["claimMappings"]["username"]["prefix"], "oidc:");
    assert_eq!(jwt["claimMappings"]["groups"]["claim"], "groups");
    assert_eq!(jwt["claimMappings"]["groups"]["prefix"], "oidc:");
}

#[test]
fn keycloak_groups_are_bound_to_cluster_roles() {
    let documents = render_with_keycloak_and_vault();
    let bindings: Vec<(String, String)> = inline_manifest(&documents, "sherpa-oidc-rbac")
        .iter()
        .map(|binding| {
            (
                binding["subjects"][0]["name"].as_str().unwrap().to_owned(),
                binding["roleRef"]["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        bindings,
        [
            ("oidc:cluster-admins".to_owned(), "cluster-admin".to_owned()),
            ("oidc:cluster-viewers".to_owned(), "view".to_owned()),
        ]
    );
}

#[test]
fn vault_clusters_install_the_operator_connected_to_their_vault() {
    let documents = render_with_keycloak_and_vault();
    let resources = inline_manifest(&documents, "vault-secrets-operator-install");
    let secret = find(&resources, "Secret");
    assert_eq!(
        secret["metadata"]["namespace"],
        "vault-secrets-operator-system"
    );
    assert_eq!(secret["stringData"]["ca.crt"], LAB_CA);
    let args: Vec<&str> =
        find(&resources, "Job")["spec"]["template"]["spec"]["containers"][0]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
    for expected in [
        "--version=1.6.2",
        "--set=defaultVaultConnection.address=https://172.31.1.24:8200",
        "--set=defaultVaultConnection.caCertSecret=sherpa-lab-ca",
        "--set=defaultAuthMethod.method=jwt",
        "--set=defaultAuthMethod.mount=kubernetes-k8s",
        "--set=defaultAuthMethod.jwt.role=workload",
        "--set=defaultAuthMethod.jwt.tokenAudiences={vault}",
    ] {
        assert!(args.contains(&expected), "missing {expected} in {args:?}");
    }
}

#[test]
fn keycloak_and_vault_leave_flannel_clusters_without_extra_manifests() {
    let documents = render_with_keycloak_and_vault();
    assert!(documents[0]["cluster"].get("extraManifests").is_none());
    assert_eq!(
        kinds(&documents),
        CONTROL_PLANE_KINDS
            .iter()
            .map(std::string::ToString::to_string)
            .chain(["KubeCoreDNSConfig".to_owned()])
            .collect::<Vec<String>>()
    );
}

#[test]
fn keycloak_clusters_get_an_oidc_login_context() {
    let output = kubeconfig(
        "k8s",
        "https://172.31.1.5:6443",
        &credentials(),
        Some(&oidc()),
    )
    .unwrap();
    let document: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(document["current-context"], "admin@k8s");
    assert_eq!(document["contexts"][1]["name"], "oidc@k8s");
    let exec = &document["users"][1]["user"]["exec"];
    assert_eq!(exec["command"], "kubectl");
    let args: Vec<&str> = exec["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    assert_eq!(&args[..2], ["oidc-login", "get-token"]);
    assert!(args.contains(&"--oidc-issuer-url=https://172.31.1.23:8443/realms/sherpa"));
    assert!(args.contains(&"--oidc-client-id=kubernetes"));
    let ca = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--certificate-authority-data="))
        .unwrap();
    assert_eq!(shared::util::base64_decode(ca).unwrap(), LAB_CA);
}
