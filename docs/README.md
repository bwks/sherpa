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
| [`investigations/`](investigations/README.md) | Debugging and research notes that are not yet settled facts. |
| [`TAILNET.md`](TAILNET.md) | Redirect stub to the docs site. Released versions link here from warnings, do not delete. |

Test runbooks live in [`../test-specs/`](../test-specs/README.md).
