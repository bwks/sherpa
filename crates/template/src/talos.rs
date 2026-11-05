//! Talos Linux machine configs for clusters declared in the manifest.
//!
//! Talos 1.14 reads a stream of configuration documents: a legacy `v1alpha1` document plus
//! typed documents (`KubeClusterConfig`, `HostnameConfig`, ...). Talos decodes them strictly, so
//! these documents follow `talosctl gen config` for the pinned Talos version exactly, apart
//! from:
//! - no `UnattendedInstallConfig`: Sherpa boots the NoCloud disk image, already installed;
//! - no discovery service documents, so labs do not depend on `discovery.talos.dev`;
//! - a static hostname, the management subnet as the kubelet node IP subnet, and a shared
//!   (VIP) address on clusters with more than one control plane;
//! - on Cilium clusters, no Flannel document, kube-proxy disabled, the Gateway API CRDs, and
//!   inline manifests that install Cilium and its load balancer, Gateway and Hubble UI resources.
//!
//! Each document is written as JSON, separated by `---`. JSON is valid YAML and keeps every
//! value's type.

use std::net::Ipv4Addr;

use anyhow::Result;
use serde_json::{Value, json};

use shared::data::ClusterRole;
use shared::konst::TALOS_KUBERNETES_VERSION;
use shared::util::base64_encode;

/// Management interface the shared address is assigned to, and Cilium announces load balancer
/// addresses on.
const TALOS_MANAGEMENT_LINK: &str = "eth0";
/// Port of KubePrism, the on-node load balancer for the Kubernetes API.
const KUBEPRISM_PORT: u16 = 7445;
/// Cilium release installed on Cilium clusters.
const CILIUM_VERSION: &str = "1.20.4";
/// Image of the `cilium` CLI that installs Cilium.
const CILIUM_CLI_IMAGE: &str = "quay.io/cilium/cilium-cli:v0.20.2";
/// Gateway API CRDs (standard channel) for the Cilium release, applied before Cilium starts.
const GATEWAY_API_CRDS_URL: &str =
    "https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.6.3/standard-install.yaml";
/// Cilium Helm values for Talos (KubePrism as the API server, Talos's cgroup mount, no
/// SYS_MODULE, legacy host routing for Talos's host DNS) and the cluster's features.
const CILIUM_HELM_VALUES: [&str; 13] = [
    "ipam.mode=kubernetes",
    "kubeProxyReplacement=true",
    "securityContext.capabilities.ciliumAgent={CHOWN,KILL,NET_ADMIN,NET_RAW,IPC_LOCK,SYS_ADMIN,SYS_RESOURCE,DAC_OVERRIDE,FOWNER,SETGID,SETUID}",
    "securityContext.capabilities.cleanCiliumState={NET_ADMIN,SYS_ADMIN,SYS_RESOURCE}",
    "cgroup.autoMount.enabled=false",
    "cgroup.hostRoot=/sys/fs/cgroup",
    "k8sServiceHost=localhost",
    "bpf.hostLegacyRouting=true",
    "gatewayAPI.enabled=true",
    "l2announcements.enabled=true",
    "k8sClientRateLimit.qps=10",
    "hubble.relay.enabled=true",
    "hubble.ui.enabled=true",
];
/// Name of the Gateway every Cilium cluster gets, in the `default` namespace.
const CILIUM_GATEWAY_NAME: &str = "sherpa";
/// Port the Hubble UI frontend listens on in its pods.
const HUBBLE_UI_POD_PORT: u16 = 8081;

/// Secrets shared by every node in a cluster. Certificates and keys are PEM.
#[derive(Clone, Debug, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct TalosClusterSecrets {
    /// Talos OS (API) CA, Ed25519.
    pub os_ca_cert: String,
    pub os_ca_key: String,
    pub etcd_ca_cert: String,
    pub etcd_ca_key: String,
    /// Kubernetes CA.
    pub kubernetes_ca_cert: String,
    pub kubernetes_ca_key: String,
    pub aggregator_ca_cert: String,
    pub aggregator_ca_key: String,
    /// Key that signs service account tokens.
    pub service_account_key: String,
    /// Kubernetes bootstrap token used by nodes to join (`[a-z0-9]{6}.[a-z0-9]{16}`).
    pub bootstrap_token: String,
    /// Token used by Talos nodes to request certificates from trustd.
    pub trustd_token: String,
    /// Base64 key that encrypts Kubernetes secrets at rest.
    pub secretbox_encryption_secret: String,
}

/// One node's machine config inputs.
#[derive(Debug)]
pub struct TalosNodeConfig<'a> {
    pub cluster_name: &'a str,
    /// Kubernetes API endpoint, e.g. `https://10.0.0.5:6443`.
    pub endpoint: &'a str,
    pub hostname: &'a str,
    pub role: ClusterRole,
    /// Management subnet in CIDR form; the kubelet registers its address from it.
    pub management_subnet: &'a str,
    /// Shared address the control planes elect an owner for, when there are several.
    pub vip: Option<Ipv4Addr>,
    /// Let workloads run on control planes (clusters with no workers).
    pub schedulable_control_planes: bool,
    /// Cilium instead of Flannel, on clusters that use it.
    pub cilium: Option<CiliumConfig>,
    pub secrets: &'a TalosClusterSecrets,
}

/// A Cilium cluster's load balancer addresses and sizing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CiliumConfig {
    /// First and last address Cilium hands out to load balancers.
    pub load_balancer_first: Ipv4Addr,
    pub load_balancer_last: Ipv4Addr,
    /// Address of the cluster's Gateway.
    pub gateway: Ipv4Addr,
    /// Address of Hubble UI.
    pub hubble_ui: Ipv4Addr,
    /// Cilium operator replicas: one per node, up to two.
    pub operator_replicas: usize,
}

/// Client credentials issued from a cluster CA. PEM.
#[derive(Debug)]
pub struct TalosClientCredentials<'a> {
    pub ca_cert: &'a str,
    pub client_cert: &'a str,
    pub client_key: &'a str,
}

/// Render a `talosconfig` for `talosctl`, as JSON (which `talosctl` reads as YAML).
///
/// `endpoints` are the control planes' addresses; `nodes` is the default node to target.
pub fn talosconfig(
    cluster_name: &str,
    endpoints: &[String],
    nodes: &[String],
    credentials: &TalosClientCredentials,
) -> Result<String> {
    let document = json!({
        "context": cluster_name,
        "contexts": {
            cluster_name: {
                "endpoints": endpoints,
                "nodes": nodes,
                "ca": base64_encode(credentials.ca_cert),
                "crt": base64_encode(credentials.client_cert),
                "key": base64_encode(credentials.client_key),
            },
        },
    });
    Ok(format!(
        "{}
",
        serde_json::to_string_pretty(&document)?
    ))
}

/// Render a `kubeconfig` for `kubectl`, as JSON (which `kubectl` reads as YAML).
pub fn kubeconfig(
    cluster_name: &str,
    endpoint: &str,
    credentials: &TalosClientCredentials,
) -> Result<String> {
    let user = format!("admin@{cluster_name}");
    let document = json!({
        "apiVersion": "v1",
        "kind": "Config",
        "clusters": [{
            "name": cluster_name,
            "cluster": {
                "server": endpoint,
                "certificate-authority-data": base64_encode(credentials.ca_cert),
            },
        }],
        "users": [{
            "name": user,
            "user": {
                "client-certificate-data": base64_encode(credentials.client_cert),
                "client-key-data": base64_encode(credentials.client_key),
            },
        }],
        "contexts": [{
            "name": user,
            "context": { "cluster": cluster_name, "user": user },
        }],
        "current-context": user,
    });
    Ok(format!(
        "{}
",
        serde_json::to_string_pretty(&document)?
    ))
}

/// Render a node's machine config as a `---`-separated stream of JSON documents.
pub fn talos_machine_config(node: &TalosNodeConfig) -> Result<String> {
    let documents = match node.role {
        ClusterRole::ControlPlane => control_plane_documents(node),
        ClusterRole::Worker => worker_documents(node),
    };
    let mut rendered = Vec::with_capacity(documents.len());
    for document in &documents {
        rendered.push(serde_json::to_string(document)?);
    }
    Ok(format!("{}\n", rendered.join("\n---\n")))
}

fn control_plane_documents(node: &TalosNodeConfig) -> Vec<Value> {
    let secrets = node.secrets;
    let issuer = node.endpoint;
    let mut node_config = json!({
        "apiVersion": "v1alpha1",
        "kind": "KubeNodeConfig",
        "nodeIP": { "validSubnets": [node.management_subnet] },
        "labels": {
            "node-role.kubernetes.io/control-plane": "",
            "node.kubernetes.io/exclude-from-external-load-balancers": "",
        },
    });
    if !node.schedulable_control_planes {
        node_config["taints"] = json!({ "node-role.kubernetes.io/control-plane": "NoSchedule" });
    }

    let mut cluster_document = json!({
        "version": "v1alpha1",
        "debug": false,
        "persist": true,
        "machine": {
            "type": "controlplane",
            "token": secrets.trustd_token,
            "ca": {
                "crt": base64_encode(&secrets.os_ca_cert),
                "key": base64_encode(&secrets.os_ca_key),
            },
            "certSANs": [],
            "features": { "diskQuotaSupport": true },
        },
        "cluster": {
            "token": secrets.bootstrap_token,
            "etcd": {
                "ca": {
                    "crt": base64_encode(&secrets.etcd_ca_cert),
                    "key": base64_encode(&secrets.etcd_ca_key),
                },
            },
        },
    });
    if let Some(cilium) = &node.cilium {
        // Talos applies these from the control planes. It retries resources whose CRDs do not
        // exist yet, until the Gateway API CRDs are applied and Cilium has started.
        cluster_document["cluster"]["extraManifests"] = json!([GATEWAY_API_CRDS_URL]);
        cluster_document["cluster"]["inlineManifests"] = cilium_inline_manifests(cilium);
    }
    let mut documents = vec![cluster_document];
    documents.extend(common_documents(node));
    documents.push(node_config);
    documents.extend(kubelet_and_network_documents());
    documents.extend([
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeAPIServerCAConfig",
            "issuingCA": {
                "cert": secrets.kubernetes_ca_cert,
                "key": secrets.kubernetes_ca_key,
            },
        }),
        json!({ "apiVersion": "v1alpha1", "kind": "KubePrismConfig", "port": KUBEPRISM_PORT }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeAggregatorCAConfig",
            "issuingCA": {
                "cert": secrets.aggregator_ca_cert,
                "key": secrets.aggregator_ca_key,
            },
        }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeAdmissionControlConfig",
            "name": "PodSecurity",
            "configuration": {
                "apiVersion": "pod-security.admission.config.k8s.io/v1alpha1",
                "defaults": {
                    "audit": "restricted",
                    "audit-version": "latest",
                    "enforce": "baseline",
                    "enforce-version": "latest",
                    "warn": "restricted",
                    "warn-version": "latest",
                },
                "exemptions": { "namespaces": ["kube-system"], "runtimeClasses": [], "usernames": [] },
                "kind": "PodSecurityConfiguration",
            },
        }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeAuditPolicyConfig",
            "configuration": {
                "apiVersion": "audit.k8s.io/v1",
                "kind": "Policy",
                "rules": [{ "level": "Metadata" }],
            },
        }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeAuthenticationConfig",
            "configuration": {
                "anonymous": {
                    "conditions": [{ "path": "/livez" }, { "path": "/readyz" }, { "path": "/healthz" }],
                    "enabled": true,
                },
                "jwt": [],
            },
        }),
        json!({ "apiVersion": "v1alpha1", "kind": "KubeAuthorizerConfig", "name": "node", "type": "Node" }),
        json!({ "apiVersion": "v1alpha1", "kind": "KubeAuthorizerConfig", "name": "rbac", "type": "RBAC" }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeEtcdEncryptionConfig",
            "config": {
                "resources": [{
                    "providers": [{
                        "secretbox": { "keys": [{ "name": "key2", "secret": secrets.secretbox_encryption_secret }] },
                    }],
                    "resources": ["secrets"],
                }],
            },
        }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeServiceAccountConfig",
            "issuer": { "privateKey": secrets.service_account_key, "issuerURL": issuer },
        }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeAPIServerConfig",
            "image": format!("registry.k8s.io/kube-apiserver:v{TALOS_KUBERNETES_VERSION}"),
        }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeControllerManagerConfig",
            "image": format!("registry.k8s.io/kube-controller-manager:v{TALOS_KUBERNETES_VERSION}"),
        }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeSchedulerConfig",
            "image": format!("registry.k8s.io/kube-scheduler:v{TALOS_KUBERNETES_VERSION}"),
            "config": {},
        }),
        kube_proxy_config(node),
    ]);
    // Talos deploys Flannel while its document is present; Cilium clusters leave it out.
    if node.cilium.is_none() {
        documents.push(json!({ "apiVersion": "v1alpha1", "kind": "KubeFlannelCNIConfig", "backendType": "vxlan", "backendPort": 4789 }));
    }
    documents
        .push(json!({ "apiVersion": "v1alpha1", "kind": "KubeCoreDNSConfig", "enabled": true }));
    if let Some(vip) = node.vip {
        documents.push(json!({
            "apiVersion": "v1alpha1",
            "kind": "Layer2VIPConfig",
            "name": vip.to_string(),
            "link": TALOS_MANAGEMENT_LINK,
        }));
    }
    documents
}

fn worker_documents(node: &TalosNodeConfig) -> Vec<Value> {
    let secrets = node.secrets;
    let mut documents = vec![json!({
        "version": "v1alpha1",
        "debug": false,
        "persist": true,
        "machine": {
            "type": "worker",
            "token": secrets.trustd_token,
            "ca": { "crt": base64_encode(&secrets.os_ca_cert), "key": "" },
            "certSANs": [],
            "features": { "diskQuotaSupport": true },
        },
        "cluster": { "token": secrets.bootstrap_token },
    })];
    documents.extend(common_documents(node));
    documents.push(json!({
        "apiVersion": "v1alpha1",
        "kind": "KubeNodeConfig",
        "nodeIP": { "validSubnets": [node.management_subnet] },
    }));
    documents.extend(kubelet_and_network_documents());
    documents.extend([
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeAPIServerCAConfig",
            "acceptedCAs": [secrets.kubernetes_ca_cert],
        }),
        json!({ "apiVersion": "v1alpha1", "kind": "KubePrismConfig", "port": KUBEPRISM_PORT }),
    ]);
    documents
}

/// kube-proxy, disabled on Cilium clusters: Cilium replaces it.
fn kube_proxy_config(node: &TalosNodeConfig) -> Value {
    let mut config = json!({
        "apiVersion": "v1alpha1",
        "kind": "KubeProxyConfig",
        "image": format!("registry.k8s.io/kube-proxy:v{TALOS_KUBERNETES_VERSION}"),
        "config": {},
    });
    if node.cilium.is_some() {
        config["enabled"] = json!(false);
    }
    config
}

/// The manifests that make a cluster a Cilium cluster, as Talos inline manifests:
/// - `cilium-install`: a Job that runs `cilium install` once, on a control plane's host
///   network, since there is no pod network before Cilium;
/// - `cilium-load-balancer`: the cluster's load balancer addresses, announced over ARP on the
///   management network;
/// - `cilium-gateway`: the cluster's Gateway and a load balancer address for Hubble UI.
fn cilium_inline_manifests(cilium: &CiliumConfig) -> Value {
    json!([
        { "name": "cilium-install", "contents": manifest_stream(&cilium_install_resources(cilium)) },
        { "name": "cilium-load-balancer", "contents": manifest_stream(&cilium_load_balancer_resources(cilium)) },
        { "name": "cilium-gateway", "contents": manifest_stream(&cilium_gateway_resources(cilium)) },
    ])
}

/// Kubernetes resources as one `---`-separated stream of JSON documents.
fn manifest_stream(resources: &[Value]) -> String {
    let mut documents = Vec::with_capacity(resources.len());
    for resource in resources {
        documents.push(resource.to_string());
    }
    documents.join("\n---\n")
}

fn cilium_install_resources(cilium: &CiliumConfig) -> Vec<Value> {
    let mut command = vec![
        "cilium".to_owned(),
        "install".to_owned(),
        format!("--version={CILIUM_VERSION}"),
    ];
    for value in CILIUM_HELM_VALUES {
        command.push(format!("--set={value}"));
    }
    command.push(format!("--set=k8sServicePort={KUBEPRISM_PORT}"));
    command.push(format!(
        "--set=operator.replicas={}",
        cilium.operator_replicas
    ));
    vec![
        json!({
            "apiVersion": "v1",
            "kind": "ServiceAccount",
            "metadata": { "name": "cilium-install", "namespace": "kube-system" },
        }),
        json!({
            "apiVersion": "rbac.authorization.k8s.io/v1",
            "kind": "ClusterRoleBinding",
            "metadata": { "name": "cilium-install" },
            "roleRef": {
                "apiGroup": "rbac.authorization.k8s.io",
                "kind": "ClusterRole",
                "name": "cluster-admin",
            },
            "subjects": [{
                "kind": "ServiceAccount",
                "name": "cilium-install",
                "namespace": "kube-system",
            }],
        }),
        json!({
            "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": { "name": "cilium-install", "namespace": "kube-system" },
            "spec": {
                "backoffLimit": 10,
                "template": {
                    "metadata": { "labels": { "app": "cilium-install" } },
                    "spec": {
                        "restartPolicy": "OnFailure",
                        "serviceAccountName": "cilium-install",
                        "hostNetwork": true,
                        // Without a pod network there is no cluster DNS yet.
                        "dnsPolicy": "Default",
                        // Nodes are not ready until Cilium runs.
                        "tolerations": [{ "operator": "Exists" }],
                        "affinity": {
                            "nodeAffinity": {
                                "requiredDuringSchedulingIgnoredDuringExecution": {
                                    "nodeSelectorTerms": [{
                                        "matchExpressions": [{
                                            "key": "node-role.kubernetes.io/control-plane",
                                            "operator": "Exists",
                                        }],
                                    }],
                                },
                            },
                        },
                        "containers": [{
                            "name": "cilium-install",
                            "image": CILIUM_CLI_IMAGE,
                            "env": [
                                { "name": "KUBERNETES_SERVICE_HOST", "value": "localhost" },
                                { "name": "KUBERNETES_SERVICE_PORT", "value": KUBEPRISM_PORT.to_string() },
                            ],
                            "command": command,
                        }],
                    },
                },
            },
        }),
    ]
}

fn cilium_load_balancer_resources(cilium: &CiliumConfig) -> Vec<Value> {
    vec![
        json!({
            "apiVersion": "cilium.io/v2",
            "kind": "CiliumLoadBalancerIPPool",
            "metadata": { "name": "sherpa" },
            "spec": {
                "blocks": [{
                    "start": cilium.load_balancer_first.to_string(),
                    "stop": cilium.load_balancer_last.to_string(),
                }],
            },
        }),
        json!({
            "apiVersion": "cilium.io/v2alpha1",
            "kind": "CiliumL2AnnouncementPolicy",
            "metadata": { "name": "sherpa" },
            "spec": {
                "loadBalancerIPs": true,
                "interfaces": [format!("^{TALOS_MANAGEMENT_LINK}$")],
            },
        }),
    ]
}

fn cilium_gateway_resources(cilium: &CiliumConfig) -> Vec<Value> {
    vec![
        json!({
            "apiVersion": "gateway.networking.k8s.io/v1",
            "kind": "Gateway",
            "metadata": { "name": CILIUM_GATEWAY_NAME, "namespace": "default" },
            "spec": {
                "gatewayClassName": "cilium",
                "addresses": [{ "type": "IPAddress", "value": cilium.gateway.to_string() }],
                "listeners": [{
                    "name": "http",
                    "protocol": "HTTP",
                    "port": 80,
                    "allowedRoutes": { "namespaces": { "from": "All" } },
                }],
            },
        }),
        json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": {
                "name": "hubble-ui-sherpa",
                "namespace": "kube-system",
                "annotations": { "lbipam.cilium.io/ips": cilium.hubble_ui.to_string() },
            },
            "spec": {
                "type": "LoadBalancer",
                "selector": { "k8s-app": "hubble-ui" },
                "ports": [{ "name": "http", "port": 80, "targetPort": HUBBLE_UI_POD_PORT }],
            },
        }),
    ]
}

/// Documents every node carries, in `talosctl gen config` order.
fn common_documents(node: &TalosNodeConfig) -> Vec<Value> {
    vec![
        json!({
            "apiVersion": "v1alpha1",
            "kind": "ResolverConfig",
            "hostDNS": { "enabled": true, "forwardKubeDNSToHost": true },
        }),
        json!({ "apiVersion": "v1alpha1", "kind": "VolumeConfig", "name": "EPHEMERAL", "mount": { "secure": true } }),
        json!({ "apiVersion": "v1alpha1", "kind": "FilesystemTrimConfig", "interval": "168h0m0s" }),
        json!({ "apiVersion": "v1alpha1", "kind": "SecurityProfileConfig", "workloadIsolation": true }),
        json!({ "apiVersion": "v1alpha1", "kind": "HostnameConfig", "hostname": node.hostname }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeClusterConfig",
            "clusterName": node.cluster_name,
            "endpoint": node.endpoint,
        }),
    ]
}

fn kubelet_and_network_documents() -> Vec<Value> {
    vec![
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeletConfig",
            "image": format!("ghcr.io/siderolabs/kubelet:v{TALOS_KUBERNETES_VERSION}"),
            "config": {},
            "defaultRuntimeSeccompProfileEnabled": true,
        }),
        json!({
            "apiVersion": "v1alpha1",
            "kind": "KubeNetworkConfig",
            "dnsDomain": "cluster.local",
            "podSubnets": ["10.244.0.0/16"],
            "serviceSubnets": ["10.96.0.0/12"],
        }),
    ]
}
