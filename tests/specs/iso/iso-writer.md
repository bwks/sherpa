# ISO Writer — Test Specifications

> **Crate:** `crates/iso/`
> **External Dependencies:** None (writes images to temporary directories)
> **Existing Tests:** Inline `#[cfg(test)]` module in `src/writer.rs`

How the writer works is in `docs/architecture/ZTP_ISO.md`.

---

## Image structure

- Primary and Joliet volume descriptors carry the label, `CD001`, the block size and a volume size
  equal to the file size; a terminator follows; the image ends with zeroed padding `[unit]` **P0**
- Primary identifiers are relaxed: case, hyphens and dots kept, `.` added to names without one,
  `;1` on files `[unit]` **P0**
- Rock Ridge: `SP` and a `CE` pointing at the `ER` entry on the root, `NM` names, `PX` modes and
  link counts `[unit]` **P0**
- Joliet gives the same paths and contents as Rock Ridge `[unit]` **P0**
- File contents round-trip through both trees, including empty and multi-sector files `[unit]` **P0**
- A directory with hundreds of entries spans sectors without a record crossing a boundary `[unit]` **P1**
- Little- and big-endian path tables agree and list directories by level, parent and name `[unit]` **P1**
- The output file is left out when it is inside the source `[unit]` **P0**

## Refusals

Each refusal returns its `IsoError` variant `[unit]` **P0**:

- missing source (`SourceDir`, keeping the `io::Error` as its source);
- invalid volume label (`InvalidVolumeId`);
- a name over 31 characters (`NameTooLong`);
- two names that convert to the same identifier (`NameClash`);
- nesting beyond eight levels (`TooDeep`), while eight levels succeed;
- symlinks (`NotRegularFile`, naming the path).
