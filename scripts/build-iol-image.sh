#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<EOF
Usage: $(basename "$0") <iol-binary-path> <version> [--l2]

Build a Cisco IOL Docker image.

Arguments:
    iol-binary-path    Path to the IOL binary file
    version            IOL version (e.g., 17.12.01)
    --l2               Build an L2 (switching) image

Examples:
    $(basename "$0") /path/to/x86_64_crb_linux-adventerprisek9-ms.bin 17.12.01
    $(basename "$0") /path/to/x86_64_crb_linux_l2-adventerprisek9-ms.bin 17.12.01 --l2
EOF
    exit 1
}

if [[ $# -lt 2 ]]; then
    usage
fi

IOL_BINARY="$1"
VERSION="$2"
L2=false

if [[ $# -ge 3 ]] && [[ "$3" == "--l2" ]]; then
    L2=true
fi

if [[ ! -f "$IOL_BINARY" ]]; then
    echo "Error: IOL binary not found: $IOL_BINARY"
    exit 1
fi

if [[ -z "$VERSION" ]]; then
    echo "Error: Version is required"
    exit 1
fi

if [[ "$L2" == true ]]; then
    IMAGE_TAG="sherpa/cisco_iol:L2-${VERSION}"
else
    IMAGE_TAG="sherpa/cisco_iol:${VERSION}"
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IOL_DIR="${SCRIPT_DIR}/iol"

if [[ ! -d "$IOL_DIR" ]]; then
    echo "Error: IOL support files directory not found: $IOL_DIR"
    exit 1
fi

BUILD_CONTEXT="$(mktemp -d)"
trap 'rm -rf "$BUILD_CONTEXT"' EXIT

cp "$IOL_BINARY" "${BUILD_CONTEXT}/iol.bin"
cp "${IOL_DIR}/Dockerfile" "${BUILD_CONTEXT}/Dockerfile"
cp "${IOL_DIR}/entrypoint.sh" "${BUILD_CONTEXT}/entrypoint.sh"
cp "${IOL_DIR}/start-iol.sh" "${BUILD_CONTEXT}/start-iol.sh"
cp "${IOL_DIR}/config.txt" "${BUILD_CONTEXT}/config.txt"

echo "Building IOL image: ${IMAGE_TAG}"
docker build -t "$IMAGE_TAG" "$BUILD_CONTEXT"

echo ""
echo "Successfully built: ${IMAGE_TAG}"
echo "Run with: docker run --rm --privileged ${IMAGE_TAG}"
