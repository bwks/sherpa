# Client lab files and secrets

The files the `sherpa` CLI keeps in a lab's folder (`.sherpa/`: lab info, SSH config, known hosts,
kubeconfig and other secret files) and how the CLI uses them to reach the lab's nodes. Only
`ssh_access` is written so far; the area's other behaviours, such as the lab folder's layout,
known hosts and secret files, are not written yet.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `ssh_access_001_the_lab_ssh_config_reaches_every_node_directly`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real services or the network, unit only for logic with too many branches to cover above.

## `ssh_access`: the CLI reaches nodes directly, or through the server with `--ssh-jump`

**Statement:** `sherpa ssh` and SSH provisioners reach a lab's nodes directly, or through the Sherpa server with `--ssh-jump`, and fail at once with a hint when a node can't be reached.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | The lab's SSH config reaches every node directly, with no jump host | e2e |
| 002 | `sherpa ssh` connects to a node itself | e2e |
| 003 | `sherpa ssh --ssh-jump` connects only to the Sherpa server, which connects to the node | e2e |
| 004 | A provisioner (`sherpa provision`) connects to its node itself | e2e |
| 005 | A provisioner with `--ssh-jump` connects to its node through the Sherpa server | e2e |
| 006 | A node with no route from the client fails at once, not after the ready timeout, saying to use `--ssh-jump` or the lab's tailnet | integration |
| 007 | An IPv6 server address is the jump host without its URL brackets | unit |
| 008 | A provisioner may ask for the passphrase of the user's key at the node, the first hop without a jump, but never for a node password | unit |

Cases 001 to 005 are in `tests/labs/ssh-access`; 006 to 008 are Rust tests in the client crate
(`cargo test -p sherpa ssh_access_`, and `-- --ignored` for 006).
