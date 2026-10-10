# Manifest Reference

Sherpa lab manifests are TOML files.

## Per-node data interfaces

A node can override the number of data interfaces created for that instance with
`data_interface_count`:

```toml
name = "example-lab"

nodes = [
  { name = "dev01", model = "ubuntu_linux", data_interface_count = 4 },
  { name = "dev02", model = "ubuntu_linux", data_interface_count = 4 },
]

links = [
  { src = "dev01::eth4", dst = "dev02::eth4" },
]
```

`data_interface_count` counts data interfaces only. It does not include the
management interface or any reserved interfaces required by the node model.

If omitted, Sherpa uses the default interface count from the node image/model
configuration. Overrides are validated against the interface names supported by
the selected model, so requesting more interfaces than the model can name will
fail manifest validation.

## Tailscale connection

An optional `[tailscale]` section enables management access through the owner's
tailnet. Set `enabled = true` and `auth_key_env` to the name of a client-side
environment variable containing a Tailscale auth key. Secrets must not be included
in the manifest. See [Tailnet setup and route approval](TAILNET.md).

## Omarchy Linux

Use `model = "omarchy_linux"` with a preinstalled QCOW2 image that has
cloud-init installed and enabled. Import it through the same image workflow
as other Linux VMs:

```sh
sherpa server image import --model omarchy_linux --version VERSION --src /absolute/path/omarchy.qcow2
```

The source path is on the server. The model defaults to UEFI, Q35, four CPUs,
8 GiB RAM, VirtIO networking and disks, and VirtIO graphics.

```toml
name = "omarchy-lab"

[[nodes]]
name = "desktop"
model = "omarchy_linux"
version = "VERSION"
```

Sherpa uses its existing cloud-init provisioning and VM lifecycle, including
management networking, the Sherpa user, file injection, and startup scripts.
The administrator group is `wheel`, matching Arch Linux.

The Omarchy installer ISO is not a cloud-ready disk image. Prepare the installed
image with cloud-init and a clean instance state before importing it. Sherpa
does not run the ISO installer or consume its unattended configuration files.
