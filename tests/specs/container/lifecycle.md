# Container Lifecycle — Test Specifications

> **Crate:** `crates/container/`
> **External Dependencies:** Running Docker daemon
> **Existing Tests:** None

---

## Container Creation and Start

**What to test:**
- `run_container()` creates and starts container with basic config (name, image, env vars) `[integration]` **P0**
- Management network attached first (determines interface ordering) `[integration]` **P0**
- Additional networks attached after creation `[integration]` **P1**
- Volumes mounted correctly `[integration]` **P0**
- Environment variables passed to container `[integration]` **P0**
- Privileged mode enabled when requested `[integration]` **P1**
- Custom user set when specified `[integration]` **P1**
- Custom shared memory size applied `[integration]` **P1**
- Capabilities (NET_ADMIN, SYS_ADMIN, etc.) applied `[integration]` **P1**
- Container with nonexistent image fails with clear error `[integration]` **P0**
- Container with duplicate name fails gracefully `[integration]` **P0**

---

## Container Start/Stop/Pause

**What to test:**
- `start_container()` starts an existing stopped container `[integration]` **P0**
- `stop_container()` stops a running container `[integration]` **P0**
- `pause_container()` pauses a running container `[integration]` **P0**
- `unpause_container()` unpauses a paused container `[integration]` **P0**
- `kill_container()` force-kills a running container `[integration]` **P0**
- Operations on nonexistent container produce clear error `[integration]` **P0**
- Stopping an already-stopped container handled gracefully `[integration]` **P1**

---

## Container Execution

**What to test:**
- `exec_container()` runs command and returns output `[integration]` **P0**
- `exec_container_detached()` fires command without waiting `[integration]` **P1**
- `exec_container_with_retry()` retries on failure with backoff `[integration]` **P1**
- Exec on stopped container fails with clear error `[integration]` **P0**

---

## Container Removal and Listing

**What to test:**
- `remove_container()` deletes a stopped container `[integration]` **P0**
- `list_containers()` returns all containers `[integration]` **P0**
- List with filters narrows results `[integration]` **P1**
- Remove running container (should it force-stop or error?) `[integration]` **P1**

---

## Docker Daemon Unavailability

**What to test:**
- All operations fail gracefully when Docker daemon is not running `[integration]` **P0**
- Error messages clearly indicate daemon unavailability `[integration]` **P1**
