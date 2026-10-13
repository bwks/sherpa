# Sherpa Documentation

Two kinds of documentation exist for Sherpa.

- **Application architecture** lives in [`architecture/`](architecture/README.md) in this repository.
  It describes how the code is built and is written for contributors.
- **User documentation** lives in the separate `sherpa-docs` repository at `../sherpa-docs` and is
  published as the docs site. Installation, CLI reference, manifest reference, configuration and
  node guides belong there. Any change to user-facing functionality must be reviewed against that
  repository and updated there.

## In this directory

| Path | Purpose |
|---|---|
| [`architecture/`](architecture/README.md) | Server internals, API transports, P2P networking, observability. |
| [`prds/`](prds/) | Proposals for open server architecture work. |
| [`MANIFEST.md`](MANIFEST.md) | Lab manifest reference and supported manifest behavior. |
| [`CURRENT_USER.md`](CURRENT_USER.md) | Provisioning the lab owner's account in Linux VMs. |
| [`TAILNET.md`](TAILNET.md) | Per-lab Tailscale gateways. Linked from server warnings, do not move. |
| [`README-IPV6.md`](README-IPV6.md) | IPv6 behavior and configuration. |
| [`README-JUNIPER-VSRXV3.0.md`](README-JUNIPER-VSRXV3.0.md) | Juniper vSRX 3.0 notes. |
| [`UNIKERNEL-IMAGES.md`](UNIKERNEL-IMAGES.md) | Unikernel image notes. |

The feature docs listed after `prds/` are user-facing and are candidates to migrate into
`sherpa-docs`. Test runbooks live in [`../test-specs/`](../test-specs/README.md).
