# Accounts and auth

Users, login, sessions, device passwords, owner logins on nodes, and who may do what to a lab. Only the `owner_login` and `access_control` behaviours are written so far; the area's other behaviours are not written yet.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `access_control_001_an_unauthenticated_up_is_refused`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above.

## `owner_login`: each node's login is the lab owner's own account, with their keys and device password

**Statement:** each node of a lab has a login for the lab's owner, under their own name, that takes the SSH keys on their account and their device password when the node was built, with administrator rights on the node; it takes no other key, and an owner with no SSH key cannot build a node.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | An owner with no SSH keys is refused at up, and no lab is created | e2e |
| 002 | On a Linux VM the owner logs in by key with `sherpa ssh`, with their own home, bash as their shell and passwordless sudo | e2e |
| 003 | On SR Linux the owner logs in by key with `sherpa ssh`, with the `srl-admin` superuser role | e2e |
| 004 | The owner logs in to each node with their device password, with no key offered | e2e |
| 005 | A key not on the owner's account, an admin's own, is refused by the node itself, as the owner's name and as its own | e2e |
| 006 | A key rotated in the owner's profile is not pushed to a running lab's nodes on resume: the key they were built with still logs in, the new one doesn't | e2e |
| 007 | An owner with no SSH keys is refused at redeploy | e2e |

## `access_control`: an operation on a lab is carried out only for a user allowed to do it

**Statement:** an operation on a lab is carried out only for a user allowed to do it: authenticated, and the lab's owner where the operation is the owner's.
**Relied on by:** the user, the admin. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | An unauthenticated up is refused | integration |
| 002 | Only the lab's owner can redeploy, over RPC and HTTP | integration |
| 003 | A lab is owned by the user who brought it up, not one the client names | integration |
| 004 | An admin who does not own a lab can inspect it | e2e |
| 005 | An admin who does not own a lab is refused its download, and nothing is written | e2e |
| 006 | An admin who does not own a lab is refused `sherpa ssh` to its nodes, before anything connects | e2e |
| 007 | An admin who does not own a lab is refused its consoles by the server | e2e |
| 008 | An admin cannot add an SSH key to another user's account, and the account's keys are unchanged | e2e |
| 009 | An admin can destroy a lab they do not own | e2e |
