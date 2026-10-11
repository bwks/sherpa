# Personal accounts in Linux VMs

Set the flag at the top level of your lab manifest:

```toml
name = "personal-lab"
provision_current_user = true
nodes = [
  { name = "vm", model = "ubuntu_linux" },
]
```

The flag defaults to `false`. When enabled, Sherpa uses the authenticated lab
creator's server account and registered SSH public keys. Register your public
keys in your Sherpa profile before starting the lab. Keep the matching private
keys on your workstation, in your SSH agent or default OpenSSH key files.

Guest usernames must be at most 32 characters, start with a lowercase ASCII
letter or underscore, and contain only lowercase ASCII letters, digits,
underscores or hyphens. `root` and the generated `sherpa` account are reserved.
Invalid usernames, missing keys and malformed keys stop provisioning before
resources are created or an existing VM is torn down. Equivalent public keys
with different comments are provisioned once.

The account is appended to supported Linux cloud-init users, including Omarchy
and Alpine, or Flatcar Ignition users. It has `/home/<username>`, Bash, its own
authorized keys and passwordless sudo. No personal password or Sherpa login
password hash is copied into the guest. The existing Sherpa account keeps its
configuration. Windows, BSD, containers, unikernels and vendor-specific ZTP
generators keep their existing behavior. Eligible VMs cannot use custom
`ztp_config` overrides with this flag.

`sherpa ssh vm` automatically selects the saved personal username. Personal
entries retain the host aliases and ProxyJump and allow the SSH agent and
default local keys. Legacy entries keep their Sherpa identity file and permission
checks. Lab metadata records typed entries for each provisioned node in
`lab-info.toml`; older files without these entries use the legacy identity:

```toml
[[ssh_usernames]]
node_name = "vm"
username = "alice"
```

Only the current lab owner can redeploy nodes, including when the requester is
an administrator. Redeploy resolves the owner from the database and uses that
owner's current public keys. The CLI downloads refreshed SSH configuration and
metadata after successful redeploy.
Other nodes retain their saved identities. After redeploy through another client
or the web UI, run `sherpa download <lab-id> --force` from your local lab directory.
If a download fails after redeploy, the VM remains redeployed; retry the download.

Resume preserves the account and its keys. Profile changes take effect on
redeploy; there is no live key synchronization. Source images are assumed not
to contain the personal account. Existing-account migration and password
reconciliation are outside this feature.

The guest fields follow the native [cloud-init user schema](https://docs.cloud-init.io/en/latest/reference/modules.html#users-and-groups)
and [Ignition user schema](https://coreos.github.io/ignition/configuration-v3_5/).
Personal SSH entries omit `IdentitiesOnly` and `IdentityFile` restrictions,
following [OpenSSH identity selection](https://man.openbsd.org/ssh_config#IdentitiesOnly).
