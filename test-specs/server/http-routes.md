# Server HTTP Routes — Test Specifications

> **Crate:** `crates/server/` (`api/router.rs`, `api/handlers.rs`)
> **External Dependencies:** Running server with SurrealDB
> **Existing Tests:** None

---

## Public Routes (No Auth Required)

**What to test:**
- `GET /login` renders login page `[integration]` **P0**
- `GET /signup` renders signup page `[integration]` **P0**
- `POST /login` authenticates and redirects `[integration]` **P0**
- `POST /signup` creates user and redirects `[integration]` **P0**
- `GET /logout` clears session and redirects `[integration]` **P0**
- `GET /health` returns 200 OK `[integration]` **P0**
- `GET /cert` returns server TLS certificate in PEM format `[integration]` **P1**

---

## Protected Routes (Auth Required)

**What to test:**
- `GET /` renders dashboard (redirects to login if unauthenticated) `[integration]` **P0**
- `GET /labs` renders lab list for current user `[integration]` **P0**
- `GET /labs/{id}` renders lab detail page `[integration]` **P0**
- `GET /profile` renders user profile `[integration]` **P1**
- `GET /profile/password` renders password change form `[integration]` **P1**
- `GET /profile/ssh-keys` renders SSH key management `[integration]` **P1**
- `POST /profile/password` updates password `[integration]` **P1**
- `POST /profile/ssh-keys` adds SSH key `[integration]` **P1**
- All protected routes return 401/redirect when unauthenticated `[integration]` **P0**

---

## Admin Routes

**What to test:**
- `GET /admin` renders admin dashboard `[integration]` **P0**
- `GET /admin/labs` renders all labs (not just user's) `[integration]` **P0**
- `GET /admin/users` renders user management `[integration]` **P0**
- `GET /admin/node-images` renders image management `[integration]` **P0**
- Admin routes reject non-admin users with 403 `[integration]` **P0**
- Admin CRUD operations (create/update/delete users, images) `[integration]` **P1**
- `GET /admin/node-images/upload` renders upload form with model dropdown `[integration]` **P0**
- `POST /admin/node-images/upload` accepts multipart form data and imports image `[integration]` **P0**
- Upload routes reject non-admin users with 403 `[integration]` **P0**
- Upload routes reject unauthenticated requests `[integration]` **P0**

---

## API Routes

**What to test:**
- `GET /api/v1/labs` returns JSON lab list `[integration]` **P0**
- `GET /api/v1/auth/login` handles API-style login `[integration]` **P1**
- API routes return JSON content type `[integration]` **P1**
- API error responses include appropriate status codes and messages `[integration]` **P1**

---

## WebSocket Upgrade

**What to test:**
- `/ws` upgrades to WebSocket connection `[integration]` **P0**
- Non-WebSocket request to /ws rejected `[integration]` **P1**

---

## SSE Streaming

**What to test:**
- SSE endpoints stream events correctly `[integration]` **P1**
- Client disconnection terminates SSE stream `[integration]` **P2**

---

## Static Assets

**What to test:**
- Static files served from correct path `[integration]` **P2**
- Missing static files return 404 `[integration]` **P2**
