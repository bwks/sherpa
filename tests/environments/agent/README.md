# agent-2 dev server

Lab `sherpa-agent-2`, node and guest agent `agent-2`: the second of the two identical Sherpa dev
servers. It differs from [dev](../dev/README.md) only in its names, its NUMA node (0) and its CPU
pins. `provision` is a link to `../dev/provision`.

Everything about the pair is in [the dev servers README](../dev/README.md): sizing, NUMA layout,
provisioning, bringing it up from `~/labs/sherpa-agent-2`, seeding images, connecting, and
starting its guest agent with `mise run agent-bus:agent ~/labs/sherpa-agent-2 agent-2`.
