# Accounts

Sherpa server accounts authenticate users to the API, CLI and web UI. A server username is also
the guest login name of the owner's account on lab nodes, so server usernames follow
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
- `db::find_user` (and so `get_user`) normalises the name it looks up.

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
the users' own accounts, which may also be administrators.

### Creation

- `sherpad init` (`crates/server/src/init.rs`) creates `admin` if it is missing, with the
  password from the `SHERPA_ADMIN_PASSWORD` environment variable or a prompt. It then asks for the first user account, unless a non-`admin` user exists; each answer
  comes from its `SHERPA_USER_*` variable (`NAME`, `PASSWORD`, `IS_ADMIN`, `SSH_KEY`) when set,
  so `init` can run unattended. Those are read from the environment only, not the env file, so
  the user's password is never stored. It never changes an existing account, so it is safe to
  re-run.
- `sherpa login` (`crates/client/src/cmd/login.rs`) takes `SHERPA_USER_NAME` and
  `SHERPA_USER_PASSWORD` from the environment when set, otherwise it prompts.
  `sherpa server user create`, `passwd` and `device-password` always prompt for the new
  password twice and never read `SHERPA_USER_PASSWORD`, which holds the caller's own password.
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
destroy and clean. For the same reason it has no [device password](#device-password).

## SSH keys

Every lab needs its owner's SSH public key, so the owner's account on lab nodes can be
provisioned. `services::guest_account::require_ssh_key` refuses `up` and `redeploy` for a user
without a key, with an error that links to the user documentation
(`konst::SSH_KEYS_DOCS_URL`).

`guest_account::from_user` refuses an owner whose username cannot be a guest login (a legacy
name, or one reserved on guests) with `User '<name>' cannot start labs`. Before 0.7.0 `up` and
`redeploy` fell back to the shared `sherpa` account; there is none now. Missing or malformed keys
are an error too, as is a missing [device password](#device-password).

How the owner's account is created on each OS, and how manifest user data reaches it, is in
[`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md).

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

## Device password

A user's device password is their login password on lab nodes (#649). From 0.7.0, `up` and
`redeploy` require it and refuse, before anything is created, when it is not set (see
[`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md#device-password)). It is one per user, sealed in
`user.device_password` (see [`DATABASE.md`](DATABASE.md#device-passwords)), never returned by
any API, and replaced rather than recovered when forgotten.

| Transport | Set | Shown |
|---|---|---|
| REST | `POST /api/v1/admin/users/{username}/device-password` | `device_password_set` in `GET /api/v1/admin/users[/{username}]` |
| WebSocket RPC | `user.device_password.set` | `device_password_set` in `user.info` and `user.list` |
| CLI | `sherpa server user device-password` | `Device password: set` / `not set` in `user info` and `user list` |
| Web | the Profile page's Device Password form | `set` / `not set` on the Profile page |

All of them call `services::user_device_password::set_device_password`: self only, admins
included (`auth::policy::require_self`), not for the built-in `admin`, with the caller's Sherpa
login password, and the password must meet `server::validate::validate_device_password` and
differ from the login password. The rule, one for every node model:

- 12 to 20 characters;
- letters, digits and `! @ _ - + .` only, starting with a letter or a digit;
- at least one lowercase letter, one uppercase letter, one digit and one of `! @ _ - + .`;
- not containing the username or `cisco`, in any case;
- no character three times in a row.

Each part has its own message. The rule is what the strictest models accept (Windows'
cloudbase-init takes at most 20 characters; Firepower Threat Defense, PAN-OS, ISE, Windows and
NX-OS check strength; Firepower Threat Defense refuses a character three times in a row; Cisco
devices refuse `cisco`), so a password that passes works on every model.
