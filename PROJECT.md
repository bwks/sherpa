# Project Backlog

## Web UI

### Lab Lifecycle
- [x] **Lab creation form** — Web UI form with build (nodes/links/bridges) and TOML import tabs, random name generation, SSE progress streaming.
- [ ] **Lab stop (down)** — API `POST /api/v1/labs/{id}/down` exists. Lab card "Stop" button is a non-functional stub.
- [ ] **Lab start (resume)** — API `POST /api/v1/labs/{id}/resume` exists. Lab card "Start" button is a non-functional stub.
- [ ] **Lab clean** — API `POST /api/v1/labs/{id}/clean` exists. No admin UI for force-cleaning a lab.

### Node Operations
- [ ] **Node stop** — Nodes table "Stop" button is disabled and marked "Coming Soon".
- [ ] **Node start** — Nodes table "Start" button is disabled and marked "Coming Soon".
- [ ] **Node redeploy** — API `POST /api/v1/labs/{id}/nodes/{name}/redeploy` exists. No UI to trigger it.
- [ ] **Console access** — Nodes table "Console" button is disabled and marked "Coming Soon". No web terminal or VNC integration.

### Links
- [ ] **Link impairment config** — API `POST /api/v1/labs/{lab_id}/links/{link_index}/impairment` exists. Lab detail shows links as read-only with no edit UI.

### Admin
- [ ] **Image scan** — API `POST /api/v1/images/scan` exists. Admin node-images page has no scan button.

## CLI

- [ ] **Link impairment command** — No CLI command to set/view link impairment (latency, jitter, packet loss). API exists but CLI has no way to use it.

## Server

- [ ] **Rate limiting** — Acknowledged as missing in API.md.
- [ ] **Configurable self-registration** — Signup is always enabled with no way to disable it.

## Frontend

- [ ] **Dioxus native GUI frontend** — Planned but not yet started.
