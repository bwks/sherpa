# Container Image Management — Test Specifications

> **Crate:** `crates/container/`
> **External Dependencies:** Running Docker daemon, OCI registries (for pull)
> **Existing Tests:** None

---

## Image Pull

**What to test:**
- `pull_image()` pulls image from registry successfully `[integration]` **P0**
- Progress callback receives human-readable status updates `[integration]` **P1**
- Pull of nonexistent image/tag fails with clear error `[integration]` **P0**
- Network failure during pull handled gracefully `[integration]` **P1**
- `pull_container_image()` pulls and saves to local tar.gz `[integration]` **P0**
- Gzip compression applied to saved image `[integration]` **P1**

---

## Image Listing

**What to test:**
- `list_images()` returns available Docker images `[integration]` **P0**
- `get_local_images()` returns locally available images `[integration]` **P0**
- Empty Docker daemon returns empty list `[integration]` **P1**

---

## Image Load (from tar archive)

**What to test:**
- `load_image()` loads a valid tar archive into Docker daemon `[integration]` **P0**
- `load_image()` loads a gzip-compressed tar.gz archive `[integration]` **P0**
- Progress callback receives status updates during load `[integration]` **P1**
- Loading a nonexistent file fails with clear error `[integration]` **P0**
- Loading an invalid/corrupt tar file fails with Docker error `[integration]` **P0**
- Round-trip: save image with `docker save`, load with `load_image()`, verify image present `[integration]` **P0**

---

## Image Save

**What to test:**
- `save_container_image()` exports image to tar.gz `[integration]` **P0**
- Saving nonexistent image fails with clear error `[integration]` **P0**
- Output file written to correct path `[integration]` **P1**
