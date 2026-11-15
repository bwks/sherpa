# agent-2 dev server

Lab `sherpa-agent-2`, node `agent-2`, guest agent `dev02`: the second of the two identical Sherpa dev
servers. It differs from [dev](../dev/README.md) only in its names, its NUMA node (0) and its CPU
pins. `provision` is a link to `../dev/provision`. Its login is the lab owner's account, not
`sherpa`, which is the server's no-login service account (see "Login account" there); a guest
built with the `sherpa` login is rebuilt, not converted (see "Rebuilding").

Everything about the pair is in [the dev servers README](../dev/README.md): sizing, NUMA layout,
login, provisioning, rebuilding, bringing it up from `~/labs/sherpa-agent-2`, seeding images,
connecting, and starting its guest agent with `mise run agent-bus:agent ~/labs/sherpa-agent-2 agent-2 dev02`.
