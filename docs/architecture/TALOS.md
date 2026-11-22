# Talos Linux and Kubernetes clusters

Design of Talos nodes and their clusters. Behaviour: [`tests/specs/talos.md`](../../tests/specs/talos.md).

## Node model

A Talos node has no SSH, so it is ready when its Talos API (port 50000) answers. It applies its
NoCloud `network-config` statically, with no DHCP, and ignores the `search` domain in it.

## Clusters

- **Controllers.** One or three: two are refused, as etcd needs a majority and two members
  tolerate no failure. A controller cannot be redeployed: its etcd member would need removing.
- **State.** Only what the server works out (endpoint, VIP, load balancer range) is stored as the
  plan; the rest is the lab's definition. Generated secrets stay on disk, `0600`, with the lab.
- **Cilium.** The Gateway API CRDs go in `extraManifests`, because Cilium enables its Gateway
  controller only if they exist when it starts. Talos only creates missing manifests, so nothing
  Sherpa installs is updated afterwards.
- **Keycloak and Vault.** The API server retries OIDC discovery while Keycloak is missing, so
  there is no start order. Vault checks workload tokens offline against the cluster's service
  account key, so it never calls the cluster.
- **Vault unsealing.** Vault seals itself whenever it restarts, and a sealed Vault serves no
  secrets. One key share, saved `0600` with the lab, unseals it, and `up`, redeploy, resume and the
  scanner share one unseal step (`vault::lab::apply_unseal_key`). Only `up` and redeploy
  initialise, as Vault hands out the key once and a second init would lose it. A Vault node with
  its own `ztp.config` is the user's: Sherpa never initialises, unseals or reports it. The scanner
  reads the unauthenticated seal status outside the lab's hold and unseals under it, in a `Ready`
  lab only, so it never races an `up` or redeploy initialising the same node. Why a managed Vault
  stays sealed (no saved key, an unreadable key file, a refused key) is kept in memory
  (`AppState::vault_seals`), like link states, as Vault holds the real state; it is logged when it
  changes, never on every scan, and its message never quotes the key file, as a TOML error quotes
  the line it is on.
- **Machine configs.** Talos decodes strictly, so the documents follow `talosctl gen config` for
  one pinned minor version exactly, and other versions are refused. Every key is PKCS#8. The API
  certificate holds `127.0.0.1` only if it is in `machine.certSANs`, which Sherpa leaves empty.
- **Bootstrap.** Talos accepts a second bootstrap while the first runs, so only the first
  controller is sent it, and `ALREADY_EXISTS` counts as done. A cluster not Ready in
  `ready_timeout` is a critical `up` error: the lab stays partly created, credentials saved.
- **Down.** Talos ignores ACPI, so members shut down over its API, workers first, to drain.
- **Limits.** Nodes need internet access; one Talos minor; no upgrades; two Cilium clusters a lab.
