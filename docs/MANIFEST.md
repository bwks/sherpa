# Manifest Reference

Sherpa lab manifests are TOML files.

## Lab description

Use the optional top-level `description` string to describe the lab's purpose:

```toml
name = "bgp-practice"
description = "Practice BGP peering and route policies between two routers."

nodes = [
  { name = "router01", model = "cisco_iosv" },
  { name = "router02", model = "cisco_iosv" },
]
```

Descriptions support TOML multiline strings and Unicode. Sherpa preserves them
when saving the manifest; existing manifests can omit the field.

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

## VM boot disk provisioning

VM boot disks use sparse QCOW2 overlays by default. `boot_disk_clone_method` is a
node-level setting with supported values `overlay` and `copy`:

```toml
name = "overlay-lab"
nodes = [
  { name = "vm", model = "omarchy_linux", version = "VERSION" },
  { name = "copied", model = "ubuntu_linux", boot_disk_clone_method = "copy" },
]
```

Omitting the setting on a VM selects `overlay`. Use `boot_disk_clone_method = "copy"`
on a VM to provision an independent full copy, including when its source image
is ineligible for overlays. A lab-level `boot_disk_clone_method` is rejected.
Containers and unikernels keep their existing provisioning when this setting is
omitted; explicit overlay requests for these kinds are rejected. Auxiliary disks
and configuration media are always copied.

The selected base must be a standalone, unencrypted QCOW2 image without external
data files, another backing image, or unsafe/unknown incompatible features.
Ineligible bases fail before startup creates resources or redeploy removes
the existing VM. Imported images must remain immutable and at the same absolute
path while overlays exist.

An omitted `boot_disk_size` inherits the base's virtual capacity. An explicit size
in GiB may equal or exceed that capacity; it cannot shrink the base. Only the new
overlay receives the requested capacity. Existing disks are never converted.
Resume preserves writes. Redeploy replaces the node disk using the requested
method and the image version pinned in the database at creation.

Image deletion is blocked by database node references and by backing paths in
managed disks, including stopped VMs and orphaned overlays. Unreadable dependency
metadata blocks deletion. Failed VM/disk cleanup retains lab records and saved
configuration so cleanup can be retried.

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
