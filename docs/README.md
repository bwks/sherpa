# Sherpa Documentation

This directory contains operator, API, and architecture documentation for Sherpa.

## Core docs

| Document | Purpose |
|---|---|
| [`SERVER.md`](SERVER.md) | Application server architecture, startup flow, routing, auth, services, TLS, scanner, and observability. |
| [`API.md`](API.md) | API transports, generated unified spec, OpenAPI endpoint, auth levels, streaming, and error formats. |
| [`MANIFEST.md`](MANIFEST.md) | Lab manifest reference and supported manifest behavior. |
| [`OTEL.md`](OTEL.md) | OpenTelemetry tracing and metrics configuration. |

## Networking and platform docs

| Document | Purpose |
|---|---|
| [`README-IPV6.md`](README-IPV6.md) | IPv6 behavior and configuration. |
| [`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md) | Point-to-point link architecture. |
| [`README-JUNIPER-VSRXV3.0.md`](README-JUNIPER-VSRXV3.0.md) | Juniper vSRX 3.0 notes. |

## Images and tests

| Document | Purpose |
|---|---|
| [`UNIKERNEL-IMAGES.md`](UNIKERNEL-IMAGES.md) | Unikernel image notes. |
| [`integration-test-plan.md`](integration-test-plan.md) | Integration test plan. |

## Generated API metadata

When `sherpad` is running:

- Unified Sherpa API spec: `GET /api/v1/spec`
- OpenAPI 3.1: `GET /api/v1/openapi.json`
- Swagger UI: `GET /api/docs`
