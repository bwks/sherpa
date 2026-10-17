use serde_json::Value;

use shared::data::ClusterRole;
use template::{
    TalosClientCredentials, TalosClusterSecrets, TalosNodeConfig, kubeconfig, talos_machine_config,
    talosconfig,
};

/// The documents of `talosctl gen config` for Talos 1.14, without the install and discovery
/// documents.
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

fn render(role: ClusterRole, vip: Option<&str>, schedulable: bool) -> Vec<Value> {
    let secrets = secrets();
    let node = TalosNodeConfig {
        cluster_name: "k8s",
        endpoint: "https://172.31.1.5:6443",
        hostname: "cp1",
        role,
        management_subnet: "172.31.1.0/24",
        vip: vip.map(|ip| ip.parse().unwrap()),
        schedulable_control_planes: schedulable,
        secrets: &secrets,
    };
    let output = talos_machine_config(&node).unwrap();
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

fn find<'a>(documents: &'a [Value], kind: &str) -> &'a Value {
    documents
        .iter()
        .find(|document| document["kind"] == kind)
        .unwrap()
}

#[test]
fn control_plane_config_matches_the_talos_document_set() {
    let documents = render(ClusterRole::ControlPlane, None, false);
    let mut expected: Vec<String> = CONTROL_PLANE_KINDS
        .iter()
        .map(|kind| kind.to_string())
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
    let documents = render(ClusterRole::Worker, None, false);
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
    let documents = render(ClusterRole::ControlPlane, Some("172.31.1.5"), true);
    let vip = find(&documents, "Layer2VIPConfig");
    assert_eq!(vip["name"], "172.31.1.5");
    assert_eq!(vip["link"], "eth0");
    assert!(find(&documents, "KubeNodeConfig").get("taints").is_none());
    let tainted = render(ClusterRole::ControlPlane, None, false);
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
    let output = kubeconfig("k8s", "https://172.31.1.5:6443", &credentials()).unwrap();
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
