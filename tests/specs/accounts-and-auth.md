# Accounts and auth

Users, login, sessions, device passwords, owner logins on nodes, and who may do what to a lab. Only the `access_control` behaviour is written so far; the area's other behaviours are not written yet.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `access_control_001_an_unauthenticated_up_is_refused`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above.

## `access_control`: an operation on a lab is carried out only for a user allowed to do it

**Statement:** an operation on a lab is carried out only for a user allowed to do it: authenticated, and the lab's owner where the operation is the owner's.
**Relied on by:** the user, the admin. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | An unauthenticated up is refused | integration |
| 002 | Only the lab's owner can redeploy, over RPC and HTTP | integration |
| 003 | A lab is owned by the user who brought it up, not one the client names | integration |
