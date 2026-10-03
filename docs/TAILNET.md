# Connect a lab to Tailscale

Sherpa can create a dedicated Tailscale subnet router for each lab. It exposes the
lab's IPv4 and IPv6 management addresses to the lab owner's tailnet. Lab nodes do
not need a Tailscale agent.

## Server configuration

The server's `/opt/sherpa/config/sherpa.toml` supports these gateway defaults:

```toml
[tailscale]
image = "tailscale/tailscale:v1.102.3"
socket_path = "/run/tailscale/tailscaled.sock"
exec_timeout_secs = 75
daemon_ready_timeout_secs = 20
enrollment_timeout_secs = 60
connection_timeout_secs = 30
stop_timeout_secs = 10
```

Newly generated server configurations include this section. Existing files may
omit it; partial sections override only the listed fields. Restart `sherpad` after
editing the file. Timeouts must be positive seconds, and the exec timeout should
exceed the enrollment timeout. The socket path is inside the container and must
be an absolute Linux Unix socket path. Docker ownership label keys remain fixed.

Image changes apply to newly created gateways. Before changing the socket path,
destroy existing Tailscale-enabled labs and recreate them after restarting the
server so the daemon and CLI use the same path.

## Manifest

Add this section to `manifest.toml`:

```toml
[tailscale]
enabled = true
auth_key_env = "SHERPA_TAILSCALE_KEY"
```

Set the named environment variable in the shell that runs `sherpa up`, using a
Tailscale **auth key** from your own tailnet. Supply the real value through your
secret manager or shell's secure input facility. Do not put the key in the
manifest. OAuth client secrets are not supported.

The CLI resolves the key locally and sends it separately over WSS with certificate
validation enabled. REST callers must supply `tailscale_auth_key` separately in
the authenticated HTTPS creation request; the server never resolves client
variable names. Web UI enrollment is not supported in this version.

An omitted section, or `enabled = false`, leaves the lab disconnected from Tailscale.
Newly saved resolved manifests use TOML; older saved JSON manifests remain readable
for redeploy. The auth key is required only for initial creation. Use a non-ephemeral key so the
gateway identity survives normal lab shutdown. If your tailnet requires device
approval, use a pre-authorized key or complete device approval during enrollment.

## Route approval

Joining the tailnet and approving subnet routes are separate operations.

For unattended creation:

1. Create a tag such as `tag:sherpa-lab` in your tailnet and authorize its owners.
2. Configure `autoApprovers.routes` in your **Tailscale policy**, allowing that tag
   to advertise the management address pools configured on your Sherpa server.
3. Generate the auth key with that tag attached.
4. Create the lab. Sherpa advertises only its allocated management subnets.

The policy lives in Tailscale, not in the Sherpa manifest. Sherpa does not change
it or require a Tailscale administrative API credential. An authorized larger
prefix also permits advertisements of subnets within that prefix.

Alternatively, open the gateway device in the Tailscale admin console and approve
its advertised IPv4 and IPv6 routes manually.

If route approval is pending, **lab creation succeeds with a warning** and a link
to this page. If Sherpa cannot verify approval, it reports that uncertainty rather
than claiming the routes are usable. `sherpa inspect` refreshes this status. Approval
can settle shortly after enrollment; inspect again after updating the policy.
No warning is needed once every advertised route is approved.

Route approval does not grant access by itself. Configure Tailscale grants/ACLs
for the intended users and management destinations. Client machines must accept
subnet routes (Linux clients generally require explicitly enabling this).

References: [Tailscale subnet routers](https://tailscale.com/docs/features/subnet-routers),
[Tailscale auth keys](https://tailscale.com/docs/features/auth-keys).

## Lifecycle and access

- `sherpa up` creates the gateway and joins the tailnet. Enrollment failure fails
  startup and invokes the existing lab rollback. Route approval warnings do not.
- Full `sherpa down` stops the gateway; full `sherpa resume` starts it using its
  persistent identity before resuming nodes. Individual node operations leave the
  gateway alone.
- `sherpa inspect` shows gateway state, route approval, advertised management
  networks, Tailscale addresses, and warnings.
- Destroy and administrative cleanup remove the gateway and its identity volume
  before removing lab networks. If either removal fails, Sherpa retains the lab's
  database ownership and saved configuration so cleanup can be retried.

Connect directly to the management IPs reported by Sherpa, using the node's normal
SSH/HTTPS credentials. Existing generated SSH configurations still use their
configured Sherpa jump host. Automatic DNS names for individual lab nodes are not
part of this feature.

Each gateway uses its own Docker network namespace, the lab's management network,
and a protected identity volume. Its management address is offset 3 in each subnet,
below the node allocation range. It uses `/dev/net/tun` and `NET_ADMIN`/`NET_RAW`,
without host networking or full privileged mode. The host must support nftables;
the gateway uses Tailscale's nftables backend and installs forwarding restrictions
through the image's `iptables-nft`/`ip6tables-nft` tools. Forwarding restrictions permit
only that lab's management subnets; the gateway uses source NAT. The official
container image is selected by the server configuration and pulled if absent.
Enrollment credentials travel over Docker exec stdin into a temporary in-memory
file, which is removed after enrollment. They are not stored in Docker environment
variables, exec arguments, manifests, or database records. Tailscale's persistent
node identity is itself sensitive and remains in the protected Docker volume.

## Recovery

- **Missing environment variable:** set the variable named by `auth_key_env` on
  the client, then retry creation.
- **Invalid/expired auth key or enrollment timeout:** check the key, device approval,
  outbound connectivity, and the server's Docker/TUN availability. Startup rolls back.
  After cleanup succeeds, retry with a fresh key. A one-use key may already have
  been consumed by an interrupted enrollment.
- **Gateway requires reauthentication on resume:** existing lab resources remain
  intact. A server administrator can authenticate the existing gateway through an
  interactive Tailscale login inside the container. The owner may instead destroy
  and recreate the lab with a fresh key when its data is no longer needed. Sherpa
  does not retain enrollment keys for automatic reauthentication.
- **Pending routes:** follow the approval instructions above; a restart is unnecessary.
- **Routes approved but unreachable:** check tailnet access policy, client route
  acceptance, node state, and address conflicts. Approval alone is not an end-to-end
  connectivity test.
- **Address conflicts:** management pools must not overlap other networks advertised
  into the same tailnet or the connecting client's local networks. Sherpa allocates
  unique subnets on one server, but cannot discover conflicts across tailnets or
  separate Sherpa servers. Configure distinct management pools before creating labs.
  4via6 and automatic renumbering are not supported.
- **Partial cleanup:** retry the normal destroy/administrator clean operation. Sherpa
  retains database ownership and saved configuration when gateway or identity volume
  removal fails, so the same owner can retry destroy. Other infrastructure may
  already have been removed. Sherpa verifies exact gateway ownership labels and
  refuses to delete unrelated resources.
- **Old Tailscale device records:** deleting local state does not guarantee that the
  device record disappears from Tailscale. Remove stale gateway devices through the
  Tailscale admin console; no administrative Tailscale key is stored by Sherpa.
