# Talos

Kubernetes clusters built from `talos_linux` nodes, and the Keycloak and Vault nodes they use: the addresses a cluster gets, its secrets, the machine config each member boots with, the bootstrap and when a cluster counts as ready, the kubeconfig and talosconfig the owner is given, Cilium, the `sherpa` realm of a Keycloak node and the cluster login through it, the initialisation of a Vault node and the secrets a cluster's workloads read from it, and what down, resume and redeploy do to a cluster. All of it runs on the server: sherpad generates the secrets and configs, calls the Talos, Kubernetes, Keycloak and Vault APIs over TLS, and keeps what it made in the lab's folder and the database. The client's part is `sherpa validate`, saving the files it is sent and printing where they are.

A cluster is a `[[talos_clusters]]` entry of `manifest.toml`: a name, its `controllers` and `workers` by node name, a `cni`, and optionally the `keycloak_server` node users log in through (`oidc`) and the `hashicorp_vault` node workloads read secrets from (`vault`). Parsing and checking that entry (names, one or three controllers, members that are defined `talos_linux` nodes listed once, `oidc` and `vault` naming nodes of the right model, no more clusters than there are addresses) is in the Manifests spec; here are only the two refusals that follow from Sherpa writing the config itself (`machine_config_007`, `008`). Other boundaries:

- The lab operations themselves, and a lab left partly created, are in [lab lifecycle](lab-lifecycle.md). The lab's folder on the server going with the lab is `lab_destroy`.
- How a VM boots and is judged ready is in [nodes](nodes.md): a Talos node is checked on its Talos API port by `node_ready_005`, and `sherpa down --node` and `sherpa resume --node` are `node_down_006` and `node_resume_005`. Here is only what differs for a cluster: its workers are judged as Kubernetes nodes (`cluster_bootstrap_006`) and its members are shut down through the Talos API (`cluster_shutdown`).
- Delivering a machine config to a node as its NoCloud `user-data`, a Talos node with the user's own config or none, and a cluster member with ZTP off (`ztp_setting_014`) are in [ZTP](ztp.md), and so is who can read a node's config on the server (`config_privacy`). The lab CA and the certificates Keycloak and Vault serve are `config_delivery_028` and `029`.
- Where the kubeconfig, the talosconfig and the Keycloak and Vault credential files are saved on the user's machine, who can read them, and the Keycloak administrator and realm users Sherpa generates and keeps are `lab_credentials` and `lab_file_privacy` in [client lab files](client-lab-files.md). Here is what a kubeconfig and a talosconfig hold, what the realm is made of, and how a Vault node gets its root token and unseal key.
- A Keycloak or Vault container's start settings are in nodes; downloading a Talos image is `image_download_008` in [images](images.md); a Talos node having no host in the lab's SSH config is `ssh_config_hosts_004`.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `cluster_bootstrap_001_a_cluster_with_one_controller_comes_up_healthy`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above. Cases marked "real cluster" need Talos VMs that form a cluster, which only an e2e run on a host with KVM gives; what they need is at the end. Cases marked "Docker" run one Vault or Keycloak container and no VM.

## `talos_node`: a Talos Linux node gets a disk it can install on and is powered off when Sherpa cannot ask it to stop

**Statement:** a `talos_linux` node, in a cluster or not, gets a 20 GB boot disk unless the manifest sets a size, and one that is in no cluster Sherpa built is powered off by down, since Talos ignores the power button and Sherpa has no credentials for its API.
**Relied on by:** the user; Talos, which needs at least 10 GB. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A `talos_linux` node with no `boot_disk.size` gets 20 GB, one with a size keeps it, and a node of another model gets no size | unit |
| 002 | A Talos node that is in no cluster of the lab is powered off, with the reason | unit |
| 003 | `sherpa down` of a lab with a Talos node in no cluster reports the node powered off with that reason, not forced off after a timeout (no test yet; needs a Talos VM) | e2e |

## `cluster_addresses`: each cluster has one address for its API, and a Cilium cluster a block for its load balancers

**Statement:** a cluster's Kubernetes API is reached at its only controller's address or, with three controllers, at a shared address of the lab's management network that still answers when one controller is down; a Cilium cluster also gets four load balancer addresses; both are given in manifest order from addresses nothing else in the lab uses, and are kept with the lab for every later operation.
**Relied on by:** the user, `kubectl`, the cluster's own nodes, which join through the address; down, redeploy and download, which read the stored plan. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | With one of three controllers stopped, the API still answers through the shared address, and once the controller is resumed the cluster is healthy again (real cluster) | e2e |
| 002 | A cluster with one controller is reached at that controller's address and has no shared address | unit |
| 003 | Clusters with three controllers get the shared addresses `.11`, then `.12`, in manifest order | unit |
| 004 | Cilium clusters get the blocks `.13` to `.16`, then `.17` to `.20`, in manifest order, the Gateway's address first and Hubble UI's second, and a Flannel cluster gets none | unit |
| 005 | A cluster's plan names its Keycloak and its Vault node with their addresses, and a node it names that has no address fails the planning | unit |
| 006 | The shared and load balancer addresses lie above the lab's infrastructure addresses and below its node addresses, and the load balancer range divides into whole blocks | unit |
| 007 | A stored plan reads back as it was planned, the clusters in manifest order | integration |
| 008 | Storing a lab's plans again replaces them | integration |
| 009 | A plan for a cluster the lab's definition does not have is refused, naming it, and nothing is stored | integration |
| 010 | A plan whose member has no management address is refused when it is read, naming the node | integration |
| 011 | A later operation gets each cluster from its stored plan and its saved secrets, and finds a member with its role and address | unit |
| 012 | Destroying the lab removes its plans | integration |
| 013 | Two clusters with three controllers in one lab both come up, each at its own shared address (no test yet; real cluster) | e2e |

## `cluster_secrets`: a cluster's secrets are generated on the server and never leave it

**Statement:** up generates for each cluster its own certificate authorities, signing key, join tokens and encryption key, saves them in the lab's folder on the server in a file only sherpad reads, uses the saved ones for every later operation, never sends them to the client, and they go with the lab's folder when the lab is destroyed.
**Relied on by:** the cluster, whose every certificate chains to them; down and redeploy; the user, against the other accounts of the server. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The generated keys are PKCS#8, the two tokens are `[a-z0-9]{6}.[a-z0-9]{16}` and differ, and the encryption key is 32 bytes in base64 | unit |
| 002 | Saved secrets read back as they were generated, from a file with mode `600` | unit |
| 003 | Two clusters of one lab get different authorities, keys and tokens (no test yet) | unit |
| 004 | What up and download send for a cluster holds no authority's private key, no token and no encryption key (no test yet) | unit |
| 005 | After up, the cluster's folder on the server and its `secrets.toml`, `talosconfig` and `kubeconfig` are readable by sherpad's account alone, and after `sherpa destroy` the folder is gone (no test yet; real cluster) | e2e |

A worker's machine config carrying no private key is `machine_config_002`.

## `machine_config`: every member of a cluster boots with a machine config Sherpa generates

**Statement:** each member of a cluster boots with a Talos machine config generated for its role from the cluster's name, address and secrets, in the form the supported Talos release decodes, so a member may not bring a config of its own and a member on another Talos release is refused.
**Relied on by:** Talos, which refuses a config with an unknown or misplaced field; the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A controller's config is the document set of `talosctl gen config` for Talos 1.14 without the install and discovery documents, with the cluster's name, address, tokens and authorities, the node's name as its hostname and the management subnet as the kubelet's | unit |
| 002 | A worker's config has the worker's documents and none of the cluster's private keys | unit |
| 003 | The controllers of a cluster with a shared address announce it on the management interface, and those of a cluster without one do not | unit |
| 004 | Controllers of a cluster with workers are tainted against workloads, and those of a cluster with none are not | unit |
| 005 | A Flannel cluster's controllers keep Flannel and kube-proxy and carry no manifest of Sherpa's | unit |
| 006 | A member on a Talos image that is not 1.14.x is refused, naming the node and its version; 1.14 with or without a leading `v` is taken | unit |
| 007 | A cluster member that sets its own `ztp.config` is refused, naming the cluster | unit |
| 008 | A cluster whose `vault` node sets its own `ztp.config` is refused | unit |
| 009 | Keycloak and Vault add no document to a controller's config, and nothing to the manifests Talos fetches by URL | unit |

A machine config reaching its node byte for byte as NoCloud `user-data` is `config_delivery_008`. That the generated configs form a working cluster on the pinned Talos release is `cluster_bootstrap_001` and `002`.

## `cluster_bootstrap`: up bootstraps each cluster once and waits for every member to be a Ready Kubernetes node

**Statement:** once a cluster's controllers answer, up bootstraps its etcd once, on its first controller, and waits within the lab's `ready_timeout` for every member to be a Ready Kubernetes node; workers are judged only there, and a cluster that is not ready in time leaves the lab partly created, with its credentials already handed over.
**Relied on by:** the user, who gets a cluster that works when `sherpa up` returns. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A cluster of one controller and three workers comes up with every node Ready, and `talosctl health` passes (real cluster) | e2e |
| 002 | A cluster of three controllers and a worker, with Cilium, a Keycloak and a Vault, comes up with every node Ready, and `talosctl health` passes (real cluster) | e2e |
| 003 | On a Flannel cluster, pods on different workers all answer through one node's port (real cluster) | e2e |
| 004 | A bootstrap Talos accepts, or answers as already done, counts as bootstrapped; any other answer is refused with its code and message, and so is an answer with no status | unit |
| 005 | Only a node whose `Ready` condition is true counts as ready | unit |
| 006 | A cluster's worker is not waited for on its Talos API during node readiness; a controller and a Talos node in no cluster are | unit |
| 007 | An up whose cluster is not Ready within `ready_timeout` exits non-zero, names the cluster, keeps the lab, and has saved the kubeconfig and talosconfig (no test yet; real cluster) | e2e |
| 008 | The bootstrap goes to the first controller alone, and is sent to it again until it answers (no test yet; needs a Talos API that answers from a script) | unit |
| 009 | A destroy during the bootstrap or the wait stops the up, and no credentials are written afterwards (no test yet) | unit |
| 010 | A cluster with no workers runs workloads on its controllers (no test yet; real cluster) | e2e |

## `cluster_credentials`: the kubeconfig and talosconfig give the owner full access to the cluster and to nothing else

**Statement:** the kubeconfig the owner is given reaches the cluster's address as a cluster administrator with a certificate the cluster's Kubernetes authority signed, and the talosconfig reaches every controller as a Talos administrator with one its Talos authority signed; a cluster that logs in through Keycloak has a second kubeconfig context that asks Keycloak for a token.
**Relied on by:** the user, `kubectl`, `talosctl`. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | The kubeconfig of a cluster with three controllers names the shared address, `.11`, as its server (real cluster) | e2e |
| 002 | The Talos certificate has the role `os:admin` and is signed by the cluster's Talos authority, and the Kubernetes certificate is `sherpa-admin` in `system:masters`, signed by its Kubernetes authority | unit |
| 003 | The talosconfig has one context named after the cluster, every controller as an endpoint, the first as its node, and the authority's certificate with the client's certificate and key | unit |
| 004 | The kubeconfig names the cluster's address as its server, `admin@<cluster>` as its current context, and carries the client's certificate | unit |
| 005 | A cluster with a Keycloak also has the context `oidc@<cluster>`, whose user runs `kubectl oidc-login` with the realm's issuer, the `kubernetes` client and the lab CA, and `admin@<cluster>` stays current | unit |
| 006 | A cluster with no Keycloak has the one context (no test yet) | unit |

`kubectl` and `talosctl` working with the files as they are saved is shown by every e2e case here, which uses them. The files being there after up and after download is `lab_credentials_001` and `002`.

## `cluster_cilium`: a cluster with `cni = "cilium"` runs Cilium with a Gateway and Hubble UI on the management network

**Statement:** a cluster that sets `cni = "cilium"` runs Cilium in place of Flannel and kube-proxy, with a Gateway named `sherpa` at the first of its load balancer addresses, Hubble UI at the second, and the rest left for the user's `LoadBalancer` services, all answered on the lab's management network; up and download print the two addresses.
**Relied on by:** the user; workloads behind the Gateway. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | Cilium runs on the cluster with no kube-proxy and no Flannel, and its install Job succeeded (real cluster) | e2e |
| 002 | On a Cilium cluster, pods on different nodes all answer through one node's port (real cluster) | e2e |
| 003 | Up prints the Gateway's address, `.13`, and a workload with an `HTTPRoute` answers there from another node of the lab (real cluster) | e2e |
| 004 | Up prints Hubble UI's address, `.14`, and it answers there (real cluster) | e2e |
| 005 | A Cilium cluster's controllers carry no Flannel document, turn kube-proxy off, fetch the Gateway API CRDs and carry the three Cilium manifests | unit |
| 006 | A worker's config is the same on a Cilium cluster as on a Flannel one | unit |
| 007 | Cilium is installed by a Job on a controller's host network, with the node's DNS, tolerating nodes that are not ready, at the pinned release with the settings Talos needs | unit |
| 008 | The load balancer pool is the cluster's block, announced on the management interface | unit |
| 009 | The Gateway takes HTTP on port 80 at the first address, and Hubble UI is a `LoadBalancer` service at the second | unit |
| 010 | A cluster runs one Cilium operator for each node, and no more than two (no test yet) | unit |
| 011 | A `LoadBalancer` service of the user's gets one of the two addresses left and answers there (no test yet; real cluster) | e2e |

## `keycloak_realm`: every Keycloak node gets the `sherpa` realm, with the lab's owner in it

**Statement:** once a `keycloak_server` node answers, Sherpa logs in to it as the node's saved administrator, over TLS that trusts the lab CA alone, and makes the realm `sherpa` with the public client `kubernetes`, the groups `cluster-admins` and `cluster-viewers`, and the lab's owner as a user in `cluster-admins` with the password saved for them; what is already there is left as it is, and a node whose realm cannot be made leaves the lab partly created.
**Relied on by:** the user; the clusters that name the node in `oidc`; the user's own applications. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After up of a lab with a Keycloak node and no cluster, the owner gets a token from the realm's issuer with the user and password in the saved file (no test yet) | e2e |
| 002 | A realm user is made a member of `cluster-admins`, with a password that is not temporary | unit |
| 003 | The `kubernetes` client is public and puts the user's groups and itself, as the audience, in its tokens | unit |
| 004 | On a real Keycloak, the realm, the client, the two groups and the user are there afterwards, and making them a second time changes nothing and fails nothing (no test yet; Docker) | integration |
| 005 | A Keycloak that does not answer within the timeout, or refuses the saved administrator, is reported naming the node, and the lab is partly created (no test yet; Docker) | integration |
| 006 | A Keycloak whose certificate the lab CA did not sign is not logged in to (no test yet; Docker) | integration |

## `cluster_oidc`: a cluster with `oidc` takes the logins of its Keycloak's realm

**Statement:** a cluster that names a Keycloak node in `oidc` accepts the tokens of that node's `sherpa` realm for the `kubernetes` client, takes the user as `oidc:<username>` and their groups as `oidc:<group>`, gives `oidc:cluster-admins` the role `cluster-admin` and `oidc:cluster-viewers` the role `view`, and gives any other realm user nothing.
**Relied on by:** the user and the people they add to the realm. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | The lab's owner logs in with `kubectl oidc-login` and the `oidc@<cluster>` settings, is `oidc:<owner>` in `oidc:cluster-admins`, and lists the nodes and every namespace's pods (real cluster) | e2e |
| 002 | A realm user in no group logs in as `oidc:<user>` and is refused as forbidden (real cluster) | e2e |
| 003 | The controllers accept tokens from the realm's issuer for the audience `kubernetes`, checked against the lab CA, with the username and the groups prefixed `oidc:` | unit |
| 004 | `oidc:cluster-admins` is bound to `cluster-admin` and `oidc:cluster-viewers` to `view` | unit |
| 005 | A cluster with no Keycloak accepts no tokens | unit |
| 006 | The issuer is the realm `sherpa` at the Keycloak node's HTTPS address, with the lab CA | unit |
| 007 | A member of `cluster-viewers` reads resources and is refused a change (no test yet; real cluster) | e2e |

## `vault_init`: a Vault node is initialised once and unsealed, and its unseal key is never lost quietly

**Statement:** Sherpa initialises each `hashicorp_vault` node that runs the config Sherpa generates, with one unseal key, saves the key and the root token before it unseals the node, and waits until the node is active; it never sends a second init to a Vault that is initialised, and when it has no key for one it says so and how to reset the node, and resets nothing itself. After resume starts such a node again, and whenever the scanner finds it running and sealed, Sherpa unseals it with the saved key without initialising anything, and one it cannot unseal shows on its node in `sherpa inspect` and the web UI, as `Vault sealed: <reason>`, until it is unsealed or stops; a Vault node with its own `ztp.config` is never touched or shown.
**Relied on by:** the user; the clusters that name the node in `vault`. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A new Vault is initialised, its root token and unseal key are saved, and it is unsealed and active (Docker) | integration |
| 002 | Run again on that Vault, Sherpa initialises nothing and returns the saved credentials (Docker) | integration |
| 003 | A Vault that is initialised with no credentials saved for it is refused as initialised outside Sherpa (Docker) | integration |
| 004 | An init that Vault completes after the request timed out is reported as a lost init response, and the credentials saved for the node's earlier Vault are left as they are (Docker) | integration |
| 005 | The same node started again with empty storage is initialised and unsealed, and its new credentials replace the saved ones (Docker) | integration |
| 006 | Only a `hashicorp_vault` node without a config of its own is initialised | unit |
| 007 | After an init that failed, init is sent once more while Vault says it is not initialised, and no more after the second failure | unit |
| 008 | A Vault found initialised after a failed init, sealed or not, is a lost init response and is never sent init again | unit |
| 009 | The lost init response error says so, gives the cause, says Sherpa has not reset the node, and gives the `sherpa redeploy --node` command that does | unit |
| 010 | When the status cannot be read after a failed init, the error gives both causes, says the init may have completed, and gives the same command | unit |
| 011 | The config Sherpa generates has Raft storage in the container, a TLS listener on port 8200 with the node's lab certificate, the node's management address as its API and cluster addresses, and the web UI on | unit |
| 012 | After up, `vault status` with the address in the saved file and the lab CA shows the node initialised and unsealed, and the saved root token is accepted (no test yet) | e2e |
| 013 | A Vault node with its own `ztp.config` is not initialised, and no credentials file is saved for it (no test yet) | e2e |
| 014 | After `sherpa down` and `sherpa resume` the node is still unsealed (no test yet) | e2e |
| 015 | A Vault that cannot be initialised or unsealed is reported, and the lab is partly created (no test yet; Docker) | integration |
| 016 | A sealed Vault Sherpa initialised, running in a `Ready` lab, is unsealed by the scanner with its saved key, and nothing is kept against it (Docker) | integration |
| 017 | A sealed Vault Sherpa initialised is unsealed with its saved key by the step resume runs after starting it, and is active before the step returns (Docker) | integration |
| 018 | The scanner checks only running `hashicorp_vault` nodes that are not being built and run Sherpa's config by the stored definition, and resume picks by the same rule; a Vault node with its own `ztp.config` is left alone | unit |
| 019 | A sealed Vault with no saved key is kept as "no saved unseal key", one whose key file cannot be read or parsed as "key file unreadable" with the cause, which never quotes the file, and no unseal is sent to either | unit |
| 020 | A Vault that refuses the saved key is kept as "key refused" and stays sealed (Docker) | integration |
| 021 | `sherpa inspect` shows `Vault sealed: <reason>` under the state of a Vault node kept sealed, and nothing for other nodes | unit |
| 022 | The web UI's nodes table shows `Vault sealed: <reason>` under the state of a Vault node kept sealed, and nothing for other nodes | unit |
| 023 | A Vault kept sealed is logged at warn once when it is first found sealed, and again only when the reason changes, at info once when it is no longer sealed, and the unseal key and root token are never logged | unit |
| 024 | A Vault found unsealed, or unsealed with its saved key, has what was kept of it cleared; an unseal that decides nothing, such as one Vault does not answer, keeps it | unit |
| 025 | A Vault node that stops has nothing kept of it being sealed; one still running and not checked keeps it | unit |
| 026 | A resumed Vault that resume does not unseal fails its node's result with the reason, after what the start said; one it unseals says so, and its node does not fail | unit |

The saved file being readable by sherpad alone is `lab_credentials_018`.

## `cluster_vault`: a cluster's workloads read the secrets under `secret/<cluster>/` of its Vault, and no others

**Statement:** for each cluster that names a Vault node in `vault`, Sherpa gives the Vault a login that accepts the cluster's service account tokens, checked with the public half of the cluster's signing key, and a policy that reads `secret/<cluster>/` alone, and installs the Vault Secrets Operator on the cluster, connected to the Vault over TLS that trusts the lab CA, so a secret written there is synced into a Kubernetes Secret.
**Relied on by:** the user's workloads. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A secret written under `secret/<cluster>/` is synced into a Kubernetes Secret by a `VaultStaticSecret`, and the Secret follows when the value in Vault changes (real cluster) | e2e |
| 002 | The operator is installed by a Job at the pinned chart release, connected to the cluster's Vault, trusting the lab CA, and logs in with the `jwt` method on the cluster's mount as role `workload` with the audience `vault` | unit |
| 003 | A cluster's Vault is the node's HTTPS address on port 8200, and its login is mounted at `kubernetes-<cluster>` | unit |
| 004 | The cluster's policy reads the secrets under `secret/<cluster>/`, reads and lists their metadata, and names no other path | unit |
| 005 | What Vault is given of the cluster's signing key is the public key alone, and a key that cannot be read is refused | unit |
| 006 | On a real Vault, the KV engine at `secret/`, the cluster's `jwt` login with its key and issuer, its policy and the role `workload` are there afterwards, a second run fails nothing, and two clusters get a login each (no test yet; Docker) | integration |
| 007 | A workload cannot read a secret outside `secret/<cluster>/`, such as another cluster's (no test yet; real cluster) | e2e |
| 008 | A Vault that is unsealed but cannot be set up for a cluster is reported naming the cluster, and the Vault's credentials are still handed over (no test yet; Docker) | integration |

## `cluster_shutdown`: down shuts a cluster's members down through Talos, workers first, and resume brings the cluster back

**Statement:** down asks each member of a cluster to shut down through its Talos API, with the cluster's saved secrets, the workers before the controllers, waits 60 seconds for each to stop and powers off one that has not; a member Sherpa cannot ask is powered off with the reason; resume starts the members again and the cluster comes back with no new bootstrap.
**Relied on by:** the user; the cluster's workloads, which are drained and not cut off. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa down` of a cluster shuts every member down, and none is forced off (real cluster) | e2e |
| 002 | After `sherpa resume` every member is Ready again and `talosctl health` passes (real cluster) | e2e |
| 003 | A worker and a controller are each asked through the Talos API, at their own address, with their cluster's secrets | unit |
| 004 | When the lab's clusters cannot be loaded, a Talos node is powered off with the cause, and a node of another model is still shut down by the power button | unit |
| 005 | A shutdown counts as accepted only when Talos answers OK; a refusal carries its code and message | unit |
| 006 | Every controller is asked only after every worker has stopped (no test yet) | unit |
| 007 | A member whose Talos API refuses the request, or does not answer, is powered off and reported as forced, with the cause (no test yet; needs a Talos API that answers from a script) | integration |
| 008 | A cluster of three controllers is healthy again after a whole-lab down and resume (no test yet; real cluster) | e2e |

## `cluster_redeploy`: a worker, a Vault and a Keycloak can be redeployed, and a controller cannot

**Statement:** redeploying a cluster's worker rebuilds it with a machine config from the cluster's stored plan and saved secrets, and it joins again; redeploying a controller is refused before anything is removed; a redeploy whose manifest gives the node another cluster or role than the lab's stored definition does is refused before anything is removed, and the node is rebuilt with its stored cluster and role; a redeployed Vault node is initialised and unsealed again and set up again for every cluster that uses it, a redeployed Keycloak node gets its realm again with the saved users, and neither is reported running before that is done.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa redeploy --node` of a controller is refused, naming its cluster (real cluster) | e2e |
| 002 | A redeployed worker joins again, and `talosctl health` passes (real cluster) | e2e |
| 003 | Only a cluster's controller is refused; a worker and a node in no cluster go on | unit |
| 004 | A redeployed container whose Vault or Keycloak cannot be set up again fails the redeploy and is not recorded running | unit |
| 005 | A redeployed container whose Vault or Keycloak is set up again is recorded running | unit |
| 006 | After `sherpa redeploy` of a cluster's Vault, the folder's file has a new root token and unseal key, and a secret written with them is synced into the cluster (no test yet; real cluster) | e2e |
| 007 | After `sherpa redeploy` of a cluster's Keycloak, the owner logs in to the cluster with the same user and password as before (no test yet; real cluster) | e2e |
| 008 | The config a redeployed worker gets is the one up gave it (no test yet) | unit |
| 009 | A redeploy whose manifest puts the node in another cluster, gives it another role, or puts it in or out of a cluster is refused before teardown, naming the node, its stored membership and the manifest's | unit |
| 010 | A redeployed node is rebuilt with the lab's stored clusters, not the manifest's: a manifest that leaves another member out of the cluster does not change it | unit |

A redeployed Keycloak node keeping its administrator and users is `lab_credentials_013` and `014`.

## Cases that need a real cluster

`talos_node_003`, `cluster_addresses_001` and `013`, `cluster_secrets_005`, `cluster_bootstrap_001` to `003`, `007` and `010`, `cluster_credentials_001`, `cluster_cilium_001` to `004` and `011`, `cluster_oidc_001`, `002` and `007`, `cluster_vault_001` and `007`, `cluster_shutdown_001`, `002` and `008`, and `cluster_redeploy_001`, `002`, `006` and `007` need Talos VMs. `keycloak_realm_001` and `vault_init_012` to `014` are e2e too, and need only a Keycloak or a Vault container. All run on the host, in `tests/labs/talos-linux`, which holds two labs: `single`, one controller and three workers on Flannel, and `ha`, three controllers and a worker with Cilium, a Keycloak and a Vault. They need:

- a Sherpa server with KVM and the `talos_linux` 1.14, `ubuntu_linux`, `keycloak_server` and `hashicorp_vault` images. A Talos node is a 4 GB VM, so the two labs together take about 40 GB of memory;
- a route from the lab's nodes to the internet: Talos pulls the Kubernetes, Cilium and operator images, the Gateway API CRDs and the operator's Helm chart when the cluster starts;
- a node inside the lab to run `kubectl`, `talosctl` and `curl` from, as the cluster's addresses are on the lab's management network: the labs' `util` node, an Ubuntu VM that installs the tools at first boot;
- for `cluster_secrets_005`, a shell on the server to read the lab's folder; for `cluster_addresses_013`, a third lab or a second cluster in `ha`, about 12 GB more.

Nothing below e2e forms a cluster: a unit test shows what a config, a plan or a credentials file holds, and the Docker cases show Sherpa against a real Vault or Keycloak, but only a real cluster shows that Talos takes the generated config, that etcd bootstraps, and that Cilium, the Keycloak login and the Vault sync work.
