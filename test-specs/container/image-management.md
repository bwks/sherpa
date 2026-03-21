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

## Image Save

**What to test:**
- `save_container_image()` exports image to tar.gz `[integration]` **P0**
- Saving nonexistent image fails with clear error `[integration]` **P0**
- Output file written to correct path `[integration]` **P1**
