#!/usr/bin/env bash
# Install pinned, checksum-verified Kubernetes and Talos tooling on the util node.
# Runs as root from cloud-init (manifest startup_scripts).
set -euo pipefail

TALOSCTL_VERSION="v1.14.4"
TALOSCTL_SHA256="0a69e3f9b682c5cd844b78dae305f6056ceb86c5fcc6913dcea78c527ed97cee"
# Talos 1.14 deploys Kubernetes 1.37.1; kubectl matches it.
KUBECTL_VERSION="v1.37.1"
KUBECTL_SHA256="65691ff77eb6fa44c908b77a1082c9f092c3b9733b5cefabec0d1104890e21a8"

install_binary() {
    local name="$1" url="$2" sha256="$3"
    local download
    download="$(mktemp)"
    curl -fsSL --retry 5 --retry-connrefused --connect-timeout 20 -o "${download}" "${url}"
    echo "${sha256}  ${download}" | sha256sum --check --status
    install -m 0755 "${download}" "/usr/local/bin/${name}"
    rm -f "${download}"
}

install_binary talosctl \
    "https://github.com/siderolabs/talos/releases/download/${TALOSCTL_VERSION}/talosctl-linux-amd64" \
    "${TALOSCTL_SHA256}"
install_binary kubectl \
    "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/amd64/kubectl" \
    "${KUBECTL_SHA256}"

talosctl version --client
kubectl version --client
