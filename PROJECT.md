# Project Backlog

## Web UI
### Console
- [ ] **Console access** — Nodes table "Console" button is disabled and marked "Coming Soon". No web terminal or VNC integration.

### Links
- [ ] **Link impairment config** — API `POST /api/v1/labs/{lab_id}/links/{link_index}/impairment` exists. Lab detail shows links read-only with no edit UI.


## CLI

- [ ] **Link impairment command** — No CLI command to set/view link impairment (latency, jitter, packet loss). API exists but CLI has no way to use it.

## Server

- [ ] **Rate limiting** — Acknowledged as missing in API.md.
- [ ] **Configurable self-registration** — Signup is always enabled with no way to disable it.

## Frontend

- [ ] **Dioxus native GUI frontend** — Planned but not yet started.
