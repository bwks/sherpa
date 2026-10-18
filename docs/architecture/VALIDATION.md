# Validation

Validators check that user supplied data is accurate, usable and secure before anything acts on
it. They only inspect the data they are given; they do not fetch or generate data.

## Where validators live

A validator lives with the crate that owns the rule.

| Location | Holds | Used by |
|---|---|---|
| `crates/shared/src/validators/` | Rules on plain values: password strength, SSH public key format, guest usernames, Tailscale auth keys, data interface count overrides. | `server`, `client`, `db` |
| `crates/topology/src/validate/` | Checks on manifest types: duplicate devices and device count, management interface use, duplicate interface links, interface bounds, link and bridge endpoints, Talos clusters, Tailscale manifest settings. | `client` (`sherpa validate`, `sherpa up`) and `server` (`up`, `redeploy`) |
| `crates/server/src/validate/` | Server-only rules: environment variable entries, node image updates, node version resolution and image existence, Tailscale gateway settings. | `server` |

Manifest checks cannot live in `shared`, because they take `topology` types and `topology`
depends on `shared`.

A rule that only one crate enforces goes in that crate's `validate` module. When a second crate
needs it, move it to `shared::validators` (or `topology::validate` for manifest types) rather
than copying it, so the client and server can never disagree about a rule.

## Client and server

The client runs the manifest checks before sending a manifest, so mistakes are reported without
a round trip. The server runs the same checks again and is the authority: a client can be old,
modified or bypassed entirely by calling the API.

## Exceptions

`server::validate::validate_and_resolve_node_versions` also fills in each node's version from
the default node image when the manifest omits it. Resolution and validation share the lookup of
the default image per model, so they stay in one function.

Node readiness (`services::node_ops::check_node_port_ready`) is a TCP probe, not a validator, and
lives with the readiness code.
