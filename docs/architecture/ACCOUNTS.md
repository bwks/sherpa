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

- `db::create_user` and `db::upsert_user` normalise, then validate, before writing;
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
| Service | `db::create_user`, `db::upsert_user` | Authoritative check for every transport |
| Database | `user.username` ASSERT (`crates/db/src/schema/user.rs`) | Length and character rule for any write, including direct queries |

The schema does not hold the reserved list: bootstrap creates the reserved `admin` account.

## Built-in `admin` account

`db::create_admin_user` creates `admin` without the reserved-name check. Only server bootstrap
calls it (`db::seed_admin_user`, used when `SHERPA_ADMIN_PASSWORD` is set).
