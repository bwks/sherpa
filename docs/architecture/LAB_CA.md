# Lab CA

Every lab has its own certificate authority, which issues each node a TLS certificate for its
names and management addresses. Vault and Keycloak nodes serve HTTPS with theirs; Sherpa
initialises and unseals Vault over that HTTPS. The user gets the CA certificate to trust lab
nodes from their workstation.

Code: `crates/server/src/tls/lab_ca.rs` (the CA and node certificates),
`crates/server/src/tls/generator.rs` (`generate_ca`, `issue_server_certificate`),
`crates/server/src/services/vault/` (Vault init and unseal).

## The CA

`lab_ca::create_lab_ca` runs once per lab, in `up`'s ZTP phase. It generates an ECDSA P-256
CA through the generic `generator::generate_ca`, with subject `CN=Sherpa Lab CA - <lab_id>,
O=Sherpa`, valid for `LAB_CERT_VALIDITY_DAYS` (365). It writes the CA to the lab's `certs`
directory: `ca.crt` (0644) and `ca.key` (0600, readable only by the server).

`lab_ca::load_lab_ca` reads both files back. Signing a certificate needs only the issuer's
subject and key, so the generator rebuilds the CA from the key with the same `CaSpec` rather than
parsing the certificate; the Talos cluster CAs work the same way. The CA subject is written CN
first, then O, which is the order labs created before this module used, so their CAs keep
issuing certificates that chain to the original `ca.crt`.

## Node certificates

`lab_ca::issue_node_certificate` issues every container, VM and unikernel node a certificate,
written as `certs/<node>.crt` (0644) and `certs/<node>.key` (0600):

| Field | Value |
|---|---|
| Subject | `CN=<node>, O=Sherpa` |
| DNS names | `<node>`, `<node>.<lab_id>`, `<node>.<lab_id>.sherpa.lab.local` |
| IP addresses | the node's IPv4 management address, and its IPv6 one when the lab has IPv6 |
| Key usage | digital signature; extended: server auth and client auth |
| Key | ECDSA P-256 |

`up` issues them while it generates each node's ZTP. `redeploy` loads the saved CA and issues
the node a new certificate for its current addresses before it rebuilds the node, so a
redeployed node keeps one.

Unikernel nodes get a certificate, but nothing delivers it to them.

## Delivery

| Node kind | Where the files go | How |
|---|---|---|
| Containers | `/etc/sherpa/certs/{ca.crt,node.crt,node.key}` | read-only bind mounts of the server files (`generate_container_ztp`) |
| cloud-init VMs | `/etc/sherpa/certs/` | `write_files`; the CA is also added to the system trust store with cloud-init's `ca_certs` module (`cloud_init_ca_certs`) |
| Ignition VMs | `/etc/sherpa/certs/` | Ignition `storage.files` |
| Windows VMs | `C:\sherpa\certs\` | cloudbase-init `write_files` |

The mounted key keeps the server's owner and mode 0600, so a container process must run as root
to read it. Vault runs as root because Sherpa's command bypasses the image entrypoint's drop to
the `vault` user; Keycloak runs with user `0:0` (`CONTAINER_KEYCLOAK_USER`).

cloud-init skips `ca_certs` on distros where the module is not supported, such as the BSDs.

## Client copy

`UpResponse.lab_ca` and `DownloadLabResponse.lab_ca` carry `ca.crt`. The client writes it as
`.sherpa/ca/lab-ca.crt` (`LabFolder::lab_ca`) in the lab folder (`lab_files::write_lab_ca`).
`download` reads it from the lab's `certs` directory, and sends an empty string for labs created
before the lab CA.

## Vault

The generated Vault config (`template/templates/vault/vault_config.jinja`) serves the API on
8200 over TLS with the node's certificate and advertises `https://<management address>:8200`.
In the container, `VAULT_ADDR` is that address and `VAULT_CACERT` the mounted CA
(`node_ops::vault_env_vars`). A node with its own `ztp_config` keeps the old plain HTTP
environment, and Sherpa does not initialise it (`vault::sherpa_initialises_vault`).

`vault::ensure_vault_unsealed` runs after node readiness in `up`, and after a Vault node is
rebuilt in `redeploy`. It talks to the API with `vault::api::VaultClient`, which trusts only the
lab CA:

1. Poll `GET /v1/sys/seal-status` until it answers, within the lab's `ready_timeout`.
2. If Vault is not initialised, `PUT /v1/sys/init` with one key share and a threshold of one,
   then save the address, unseal key and root token as `<lab>/vault/<node>.toml` (0600),
   before unsealing, so a failure in between cannot lose them.
3. If Vault is sealed (always, right after init), `PUT /v1/sys/unseal` with the saved key, and
   fail if it is still sealed.
4. Poll `GET /v1/sys/health` every second until it answers 200, the active node. Right after
   an unseal, Raft storage elects a leader first, and other requests answer 500 ("local node
   not active") until it has.

In `up` and `redeploy`, `talos::grant_vault_access` then sets Vault up for the clusters that
use the node (see "Keycloak login and Vault secrets" in `TALOS.md`).

An initialised Vault without saved credentials (initialised outside Sherpa) is reported, not
touched. In `up`, a failure is a non-critical `UpError`; in `redeploy` it fails the operation.

Vault's Raft data is in the container, so a redeployed Vault is new: it is initialised again
and its credentials file is replaced. `down` pauses containers, so Vault stays unsealed across
`down` and `resume`.

`UpResponse.vaults` and `DownloadLabResponse.vaults` carry each Vault's `VaultInfo` (address,
root token, unseal key). The client writes them as `.sherpa/vault/vault-<node>.toml` (0600) and prints the
`VAULT_ADDR`/`VAULT_CACERT` exports (`client::vault_credentials`). After `redeploy`, the
client's `download --force` writes the new credentials.

## Keycloak

`keycloak_server` nodes run `quay.io/keycloak/keycloak` with `start` (production mode) and the
built-in file database, HTTPS only on 8443 with the node's certificate, `KC_HOSTNAME`
`https://<management address>:8443`, health endpoints on the management port 9000, and the
Sherpa user as the `master` realm's bootstrap admin (`node_ops::keycloak_env_vars`). Its data is
in the container, so a redeployed Keycloak starts empty.

After `up` (and a Keycloak `redeploy`), `keycloak::ensure_keycloak_realm` waits for the master
realm's OpenID configuration, logs in as the bootstrap admin (`admin-cli`), and makes, through
the admin API (no realm import file, which would be JSON):

- realm `sherpa`;
- client `kubernetes`: public, browser login with PKCE and direct grants, redirect addresses of
  the `kubectl oidc-login` plugin, a `groups` claim (group names, no path) and itself as an
  audience in tokens;
- groups `cluster-admins` and `cluster-viewers`, which Talos clusters bind to `cluster-admin`
  and `view` (see `TALOS.md`);
- users `sherpa` with the lab password and the lab owner with a generated password, both in
  `cluster-admins`.

A create that conflicts (HTTP 409) is left as it is. The users and passwords are saved before
they are created, in `<lab>/keycloak/<node>.toml` (0600), and reused, so a redeployed Keycloak
gets the same passwords. `UpResponse.keycloaks` and `DownloadLabResponse.keycloaks` carry
them (`KeycloakInfo`); the client writes `.sherpa/keycloak/keycloak-<node>.toml` (0600) with the address,
realm, issuer, account console and users (`client::keycloak_credentials`).
