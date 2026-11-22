# ZTP ISO images

How Sherpa builds the CD-ROM images that carry a VM node's first-boot configuration: NoCloud
seeds for cloud-init, cloudbase-init and Talos, the config ISOs network operating systems read,
and the PAN-OS bootstrap ISO. The behaviour is `config_iso` and `config_delivery` in
[`tests/specs/ztp.md`](../../tests/specs/ztp.md).

The writer is the `iso` crate, `iso::create_iso`. It writes the subset of ISO 9660 (ECMA-119)
these readers need, with Rock Ridge and Joliet, in plain Rust with no ISO crate and no external
command, so the server needs no ISO tooling. The crate depends on nothing in Sherpa, and `server`
is its only dependent, so the `sherpa` CLI does not compile it.

## From config files to the guest

1. A generator in `server::services::node_ops` writes the node's config files into its boot
   folder, `vm-boot.d/<node>/` (`node_ops::vm_boot_dir`), and calls `create_iso` with that folder
   as the source and `ztp.iso` inside it as the destination; the image leaves itself out.
2. The node's disk plan copies `ztp.iso` into the storage pool as `<node>-<lab>.iso` and attaches
   the copy as a raw CD-ROM.
3. Once every disk of the `up` or `redeploy` is in the pool, `node_ops::clone_node_disks` removes
   the boot folder, since the domain names only the pool's copy; a failed copy keeps it.
4. The guest finds the medium by label: `cidata` for NoCloud and every config ISO, `bootstrap`
   for a generated PAN-OS bootstrap. Network operating systems then look for their own file name.

The boot files are private to `sherpad` (`0600` in `0700` folders; see `config_privacy` in
[`tests/specs/ztp.md`](../../tests/specs/ztp.md)), whatever the umask: `create_iso` writes the
image `0600`, and the disk images copied from the blank disk templates are set `0600` as they are
copied. The pool copy is `0600` too, handed to QEMU by libvirt while the VM runs. The ISO's own Rock Ridge modes are fixed read-only, owned by root,
whatever the source files' modes.

## Names in three views

One image carries each name three times: the primary tree with relaxed names, Rock Ridge `NM`
with the name byte for byte, and Joliet in UTF-16. Linux reads Rock Ridge, then Joliet, then the
primary names; Windows reads Joliet; which view a network operating system reads is not
documented by its vendor. Carrying all three lets each find its file. The primary names follow
`genisoimage -l -allow-lowercase -allow-multidot -relaxed-filenames`: case, hyphens and dots survive.

Two names that convert to the same identifier in either tree are refused, not renamed: a guest
looks for an exact file name, and a renamed file is a file it never finds.

## Design decisions

- **Refuse, never approximate.** Anything the image cannot represent faithfully is an
  `IsoError` naming the path: deep folders are refused rather than relocated (Rock Ridge
  `RE`/`CL`), files of 4 GiB or more rather than split over extents, symlinks and special files
  rather than followed. Nothing Sherpa generates comes near these limits; they exist so a future
  caller gets an error instead of an image a guest cannot read.
- **Atomic.** The image is written to a hidden partial file beside the destination and renamed
  over it, so a failure leaves the previous image in place.
- **Streamed.** File data is copied from disk and never held in memory; a file that changes size
  between the scan and the copy fails the write.
- **Self-checking layout.** Positions are planned in one pass and written in another; the writer
  checks before each part that it is at the sector the plan gave it, so a mismatch is an error,
  not a corrupt image.

## Testing

The unit tests read each image back with a parser of their own, not `isoinfo` (it ships with
`genisoimage`, which the server does not install) or a mount (it needs root). After a change to
the writer, run `tests/labs/nixos-linux`, which boots cloud-init from the image end to end.
