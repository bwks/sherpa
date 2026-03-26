# Image Management End-to-End — Test Specifications

> **Scope:** Cross-crate integration testing image workflows
> **External Dependencies:** SurrealDB, Docker, filesystem, OCI registries
> **Existing Tests:** None

---

## VM Image Import

**What to test:**
- Import image from local file → copied to images dir → tracked in DB `[e2e]` **P0**
- First image for a model automatically marked as default `[e2e]` **P0**
- Subsequent imports for same model not marked as default `[e2e]` **P1**
- Import with nonexistent source file produces error `[e2e]` **P0**
- Disk path follows convention: `{images_dir}/{model}/{version}/virtioa.qcow2` `[e2e]` **P0**

---

## Image Scan

**What to test:**
- Scan discovers VM images on disk by directory structure `[e2e]` **P0**
- Scan discovers container images from Docker daemon `[e2e]` **P0**
- Discovered images bulk-upserted to DB `[e2e]` **P0**
- Dry-run mode reports discoveries without DB changes `[e2e]` **P1**

---

## Container Image Pull

**What to test:**
- Pull from OCI registry → stored in Docker → tracked in DB `[e2e]` **P0**
- Progress messages streamed during pull `[e2e]` **P1**
- Pull of nonexistent image/tag fails with error `[e2e]` **P0**
- First container image for model marked as default `[e2e]` **P1**

---

## VM Image Download

**What to test:**
- Download from URL → saved to disk → tracked in DB `[e2e]` **P1**
- Progress reported at 5MB intervals `[e2e]` **P2**
- Invalid/unreachable URL produces error `[e2e]` **P1**

---

## Set Default Version

**What to test:**
- `image.set_default` changes which version is default for a model `[e2e]` **P0**
- Previous default unset when new default applied `[e2e]` **P0**

---

## Image Deletion

**What to test:**
- Delete image → DB record removed → disk files deleted `[e2e]` **P0**
- Delete blocked if nodes reference the image (referential integrity) `[e2e]` **P0**
- Error message identifies blocking nodes `[e2e]` **P1**
- Container images: only DB record removed (Docker image not deleted) `[e2e]` **P1**

---

## Image Show

**What to test:**
- Show default image returns full NodeConfig with all fields `[e2e]` **P0**
- Show with `--version` returns details for that specific version `[e2e]` **P1**
- Show for model with no images in DB returns error `[e2e]` **P0**
- Show with nonexistent version returns error `[e2e]` **P1**
- Show available via both `sherpa image show` and `sherpa server image show` `[e2e]` **P1**

---

## Image Listing

**What to test:**
- List all images across models `[e2e]` **P0**
- Filter by model `[e2e]` **P1**
- Filter by kind (VM, container, unikernel) `[e2e]` **P1**
- Empty result when no images imported `[e2e]` **P1**
