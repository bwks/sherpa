#!/usr/bin/env bash
set -euo pipefail

TEMPLATE_CONFIG=true

# Use user-provided startup config if mounted, otherwise use built-in default
if [[ -f /iol/startup-config.txt ]]; then
    echo "Using user-provided startup config: /iol/startup-config.txt"
    cp /iol/startup-config.txt /iol/config.txt
    TEMPLATE_CONFIG=false
fi

# Flush eth0 IP — IOL takes over the interface via iouyap
ip addr flush dev eth0

# Hand off to start-iol.sh
export TEMPLATE_CONFIG
exec /iol/start-iol.sh
