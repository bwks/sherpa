# Accounts

Sherpa server accounts authenticate users to the API, CLI and web UI. A server username is also
the guest login name of the owner's personal account on lab nodes, so server usernames follow
the guest login name rules.

## Usernames

`shared::validators::validate_username` (`crates/shared/src/validators/username.rs`) is the
rule:

- 3–32 characters (`konst::USERNAME_MIN_LENGTH`, `konst::USERNAME_MAX_LENGTH`);
- lowercase ASCII letters, digits, `_` and `-`, starting with a letter or `_`;
- not in `konst::RESERVED_USERNAMES` and not starting with a prefix in
  `konst::RESERVED_USERNAME_PREFIXES`.

The reserved list covers Sherpa's own accounts (`admin`, `sherpa`), privileged and system
accounts, and the default users of common guest images and network operating systems, which
would collide with accounts that already exist on lab nodes.

### Case-insensitive usernames

`shared::util::normalize_username` trims and lowercases a username. The `db` crate applies it in
one place so callers cannot skip it:

- `db::create_user` normalises, then validates, before writing;
- `db::find_user` (and so `get_user` and `get_user_for_auth`) and `db::delete_user_by_username`
  normalise the name they look up.

Tokens carry the stored, lower-case username, so `AuthContext::username` is always canonical.
Handlers that compare a caller-supplied username with the caller's own (self-service password
change and user info on REST and WebSocket RPC, and the admin web pages) normalise the supplied
name before comparing.

### Enforcement points

| Layer | Where | Effect |
|---|---|---|
| Client | `sherpa server user create` (`crates/client/src/cmd/server/user.rs`) | Rejects an invalid name before prompting for a password |
| Web UI | `templates/auth/signup.html.jinja` | Browser `pattern` and `maxlength` hints |
| REST | `create_user_json` | Validates first and returns `400 Bad Request` |
| Service | `db::create_user` | Authoritative check for every transport |
| Database | `user.username` ASSERT (`crates/db/src/schema/user.rs`) | Length and character rule for any write, including direct queries |

The schema does not hold the reserved list: bootstrap creates the reserved `admin` account.

## Built-in `admin` account

`admin` is the server's own account, for administration and recovery. Lab work is done from
personal accounts, which may also be administrators.

### Creation

- `sherpad init` (`crates/server/src/init.rs`) creates `admin` if it is missing, with the
  password from `SHERPA_ADMIN_PASSWORD` (environment, then `/opt/sherpa/env/sherpa.env`) or a
  prompt. It then asks for the first user account, unless a non-`admin` user exists. It never
  changes an existing account, so it is safe to re-run.
- At startup, `daemon::state::ensure_admin_account` seeds `admin` when `SHERPA_ADMIN_PASSWORD`
  is set and the account is missing, and otherwise refuses to start without it.
- `db::create_admin_user` is the only way to create `admin`; `db::create_user` rejects it as a
  reserved name.

### Protection

| Layer | Where | Refuses |
|---|---|---|
| Service | `services::user_delete::check_user_delete` | Deleting `admin`, on every transport |
| Schema field assertions | `user.username`, `user.is_admin` (`crates/db/src/schema/user.rs`) | Renaming `admin`, clearing its admin flag, an `admin` record without the flag |
| Schema event | `protect_admin` on `user` | Deleting, renaming or demoting `admin`; the throw aborts the whole statement, so a bulk `DELETE user` removes nobody |

No API path renames a user or changes the admin flag; the schema guards cover application bugs
and ordinary queries. A database root user can still remove the definitions. Table permissions
do not help: they only apply to record-level users, and the server connects as a system user.

### Lab work

`auth::policy::require_lab_starter` refuses `up`, `resume` and `redeploy` for `admin`. The
services call it (`up::up_lab`, `resume::start_lab_nodes`, `redeploy::redeploy_node`), and the
REST create handler and web UI create form check it first so a refusal is a `403` or a form
error rather than a failed job. `admin` keeps every other lab operation, such as inspect, down,
destroy and clean.

## SSH keys

Every lab needs its owner's SSH public key, so the owner's personal account on lab nodes can be
provisioned. `services::guest_account::require_ssh_key` refuses `up` and `redeploy` for a user
without a key, with an error that links to the user documentation
(`konst::SSH_KEYS_DOCS_URL`).

`guest_account::from_user` returns no account, rather than an error, when the owner's username
cannot be a guest login (a legacy name, or one reserved on guests); `up` and `redeploy` report
that the nodes use the `sherpa` account instead. Missing or malformed keys stay an error.

How the personal account, or `sherpa` when there is none, is created on each OS, and how manifest
user data reaches it, is in [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md).

### Managing keys

`services::user_ssh_keys` adds and removes keys for every transport:

| Transport | Add | Remove |
|---|---|---|
| REST | `POST /api/v1/admin/users/{username}/ssh-keys` | `DELETE /api/v1/admin/users/{username}/ssh-keys` |
| WebSocket RPC | `user.ssh_key.add` | `user.ssh_key.remove` |
| CLI | `sherpa server user ssh-key add` | `sherpa server user ssh-key remove` |

Listing uses `user.info`. The rules: the caller acts on their own account, or any account as an
admin (`auth::policy::require_self_or_admin`); a key must pass
`shared::validators::validate_ssh_key` and parse as an OpenSSH key; the same key data is not
registered twice, whatever the comment; removal matches the key data or its `SHA256:`
fingerprint (`shared::util::ssh_key_fingerprint`). The web UI profile and admin pages keep their
own index-based handlers.

