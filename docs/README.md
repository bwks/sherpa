# Sherpa Documentation

This directory contains operator, API, and architecture documentation for Sherpa.

## Core docs

| Document | Purpose |
|---|---|
| [`SERVER.md`](SERVER.md) | Application server architecture, startup flow, routing, auth, services, TLS, scanner, and observability. |
| [`API.md`](API.md) | API transports, generated unified spec, OpenAPI endpoint, auth levels, streaming, and error formats. |
| [`MANIFEST.md`](MANIFEST.md) | Lab manifest reference and supported manifest behavior. |
| [`CURRENT_USER.md`](CURRENT_USER.md) | Provisioning the lab owner's account in Linux VMs. |
| [`OTEL.md`](OTEL.md) | OpenTelemetry tracing and metrics configuration. |
| [`prds/`](prds/) | Proposals for open server architecture work. |

## Networking and platform docs

| Document | Purpose |
|---|---|
| [`README-IPV6.md`](README-IPV6.md) | IPv6 behavior and configuration. |
| [`TAILNET.md`](TAILNET.md) | Per-lab Tailscale gateways. |
| [`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md) | Point-to-point link architecture. |
| [`README-JUNIPER-VSRXV3.0.md`](README-JUNIPER-VSRXV3.0.md) | Juniper vSRX 3.0 notes. |

## Images

| Document | Purpose |
|---|---|
| [`UNIKERNEL-IMAGES.md`](UNIKERNEL-IMAGES.md) | Unikernel image notes. |

Test runbooks live in [`../test-specs/`](../test-specs/README.md). Generated API metadata endpoints are listed in [`API.md`](API.md).
