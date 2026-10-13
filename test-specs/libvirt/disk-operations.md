# Disk Operations — Test Specifications

> **Crate:** `crates/libvirt/`
> **External Dependencies:** Running libvirt daemon, storage pool, disk images on filesystem
> **Existing Tests:** None

---

## Disk Cloning

**What to test:**
- `clone_disk()` clones qcow2 disk image `[integration]` **P0**
- Clone supports all formats: qcow2, iso, raw, json, ign, img `[integration]` **P0**
- Unsupported file extension rejected with error `[integration]` **P0**
- Clone from nonexistent source file fails with clear error `[integration]` **P0**
- Streaming upload uses 25MB chunk size `[integration]` **P2**
- Raw formats (iso, json, ign, img) use "raw" format type `[integration]` **P1**
- qcow2 format uses "qcow2" format type `[integration]` **P1**
- Cloned volume exists in storage pool after operation `[integration]` **P0**
- Storage pool unavailable produces clear error `[integration]` **P0**

---

## Disk Resize

**What to test:**
- `resize_disk()` increases volume size `[integration]` **P0**
- New size must be greater than current size `[integration]` **P0**
- Size specified in GB `[integration]` **P1**
- Pool refreshed before resize operation `[integration]` **P1**
- Nonexistent volume name fails with error `[integration]` **P0**
- Storage pool unavailable produces clear error `[integration]` **P0**

---

## Disk Deletion

**What to test:**
- `delete_disk()` removes volume from storage pool `[integration]` **P0**
- Deleting nonexistent volume handled gracefully `[integration]` **P1**
- Volume no longer accessible after deletion `[integration]` **P0**
