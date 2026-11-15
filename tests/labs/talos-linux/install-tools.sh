#!/usr/bin/env bash
# Install pinned, checksum-verified Kubernetes and Talos tooling on the util node: talosctl,
# kubectl and the kubelogin plugin (kubectl oidc-login).
# Runs as root from cloud-init (manifest startup_scripts).
set -euo pipefail

TALOSCTL_VERSION="v1.14.4"
TALOSCTL_SHA256="0a69e3f9b682c5cd844b78dae305f6056ceb86c5fcc6913dcea78c527ed97cee"
# Talos 1.14 deploys Kubernetes 1.37.1; kubectl matches it.
KUBECTL_VERSION="v1.37.1"
KUBECTL_SHA256="65691ff77eb6fa44c908b77a1082c9f092c3b9733b5cefabec0d1104890e21a8"
KUBELOGIN_VERSION="v1.36.4"
KUBELOGIN_SHA256="9e8baeb4905d35a301304af2548324997add512fe94ab20e65993d167212bf85"

install_binary() {
    local name="$1" url="$2" sha256="$3"
    local download
    download="$(mktemp)"
    curl -fsSL --retry 5 --retry-connrefused --connect-timeout 20 -o "${download}" "${url}"
    echo "${sha256}  ${download}" | sha256sum --check --status
    install -m 0755 "${download}" "/usr/local/bin/${name}"
    rm -f "${download}"
}

# kubelogin ships as a zip holding the binary; kubectl finds it as the oidc-login plugin.
install_kubelogin() {
    local download work
    download="$(mktemp)"
    work="$(mktemp -d)"
    curl -fsSL --retry 5 --retry-connrefused --connect-timeout 20 -o "${download}" \
        "https://github.com/int128/kubelogin/releases/download/${KUBELOGIN_VERSION}/kubelogin_linux_amd64.zip"
    echo "${KUBELOGIN_SHA256}  ${download}" | sha256sum --check --status
    python3 -m zipfile -e "${download}" "${work}"
    install -m 0755 "${work}/kubelogin" /usr/local/bin/kubectl-oidc_login
    rm -rf "${download}" "${work}"
}

install_binary talosctl \
    "https://github.com/siderolabs/talos/releases/download/${TALOSCTL_VERSION}/talosctl-linux-amd64" \
    "${TALOSCTL_SHA256}"
install_binary kubectl \
    "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/amd64/kubectl" \
    "${KUBECTL_SHA256}"
install_kubelogin

talosctl version --client
kubectl version --client
kubectl oidc-login --version
