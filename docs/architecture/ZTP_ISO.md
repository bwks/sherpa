# ZTP ISO images

How Sherpa builds the CD-ROM images that carry a VM node's first-boot configuration: NoCloud
seeds for cloud-init, cloudbase-init and Talos, the ZTP config ISOs network operating systems
read, and the PAN-OS bootstrap ISO. The writer is `create_iso` in
`crates/shared/src/util/iso.rs`; its constants are the `ISO_*` entries in
`crates/shared/src/konst.rs`. It replaced a call to the `genisoimage` command and writes the
same structure, so Sherpa needs no ISO tooling on the server.

## Callers

`crates/server/src/services/node_ops.rs` writes a node's config files into its lab directory,
then calls `create_iso` with that directory as the source and `ztp.iso` inside it as the
destination. The image leaves itself out. Labels:

| Label | Constant | Used by |
|---|---|---|
| `cidata` | `ZTP_ISO_VOLUME_ID` | NoCloud seeds and every network OS ZTP ISO |
| `bootstrap` | `PALOALTO_BOOTSTRAP_VOLUME_ID` | PAN-OS, whose source has `config/`, `content/`, `license/` and `software/` |

## What is written

One image carries the same files under three sets of names, because config readers differ in
which they use:

- **Primary tree.** Relaxed names: case, hyphens and extra dots are kept, any other character
  that is not printable ASCII becomes `_`, and files get `;1`, with a `.` first when the name
  has none (`meta-data.;1`), as `genisoimage -l -allow-lowercase -allow-multidot
  -relaxed-filenames` writes them. Names are limited to 31 characters.
- **Rock Ridge** on the primary tree: `NM` carries the real name, `PX` gives directories 0555
  and files 0444 owned by root (the rationalised form of `genisoimage -r`), `TF` the build time.
  The root's `.` record also holds `SP` and a `CE` pointing at a continuation sector with the
  `ER` (`RRIP_1991A`) entry.
- **Joliet** tree (supplementary descriptor, UCS-2 level 3): the real names in UTF-16, up to 64
  characters, with `* / : ; ? \` replaced by `_`.

Both trees point at the same file data. Sector layout, in order: 16 zeroed system-area
sectors; primary, Joliet and terminator descriptors; little- and big-endian path tables for each
tree; primary directories; the Rock Ridge continuation sector; Joliet directories; file data;
150 zeroed padding sectors. Directory records never cross a sector boundary, so a directory
can span several sectors. `SectorWriter::check_sector` checks every part lands on the sector the
layout planned.

## Limits

The writer refuses rather than guesses: symlinks, special files and non-UTF-8 names, names over
the limits above, two names that convert to the same identifier, nesting deeper than eight
levels (the root included), files of 4 GiB or more, and volume labels that are not 1 to 16
printable ASCII characters. Nothing Sherpa generates comes near these limits.

## Tests

The unit tests in `iso.rs` read the image back with a small parser: descriptors and labels,
primary, Rock Ridge and Joliet names, modes and links, path tables, file contents (an empty
file and a multi-sector one), a directory spanning sectors, the output left out of its source,
and each refusal. The research and the boot tests on every model that takes a config ISO are
on issue #2.
