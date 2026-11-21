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

How to run tests is in [`../tests/README.md`](../tests/README.md); test specs are in [`../tests/specs/`](../tests/specs/README.md).
