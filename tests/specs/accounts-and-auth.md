# Accounts and auth

User accounts and the built-in admin, login and logout on the CLI, the REST API and the web UI, sessions and their tokens, login passwords, the SSH keys on an account, device passwords, the owner's login on each node, who may do what to a lab, and what only an admin may do. Sherpa keeps no SSH key of its own for a lab: a node trusts only its owner's keys, so there is no lab key behaviour here. Who may change images is `image_access` in [images.md](images.md); who may open a console is also in the Consoles spec.

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
| 008 | An owner with no device password is refused at up, before anything is created | integration |
| 009 | An owner with no device password is refused at redeploy, before any of it starts | integration |
| 010 | A lab with a node whose model cannot take the owner's name, too long or starting with `_`, is refused, naming the node, the model and the limit; every other model takes every Sherpa username | unit |
| 011 | An owner whose name cannot be a login on a node, a reserved one such as `root` or one that breaks the login name rule, is refused with the reason | unit |
| 012 | Linux, BSD and Windows nodes take every key on the owner's account, with duplicates of one key dropped | unit |
| 013 | A network device takes only the key types it accepts and no more keys than it holds, and the keys left out are reported | unit |
| 014 | A device that none of the owner's keys fits fails, with the command that makes a key it takes | unit |
| 015 | The account on each operating system has that system's shell, home and admin rights: Bash and sudo on Linux, the system Bash on NixOS, `ash` and `doas` on Alpine, `sh` and `wheel` on the BSDs, sudo and `docker` on Flatcar, sudo on the SONiC container when its image has it, a local administrator on Windows | unit |
| 016 | The device password is written to each node as plain text or as a hash that verifies against it, as the model takes | unit |
| 017 | A node with a custom ZTP config, or with ZTP off in its manifest or its image, gets no owner login | unit |
| 018 | FRR and the SONiC container keep the owner login with a custom config, as it is made apart from the config | unit |
| 019 | At up each node records the login it is built with: the owner's, FTDv's `admin`, or none where Sherpa makes no login | unit |
| 020 | A redeployed node records its login the same way | unit |
| 021 | An owner's account shows nothing of the device password in debug output | unit |
| 022 | Each device is given the owner's keys in the form it takes: the OpenSSH key, an MD5 or SHA-256 hash of it for Cisco, a Junos statement, base64 for PAN-OS | unit |
| 023 | A generated cloud-init, cloudbase-init or Ignition config makes one account, the owner's | unit |
| 024 | A Windows node disables every local account but the owner's, before its startup scripts run | unit |
| 025 | On Windows each account has its own keys file, so the owner's keys log in only as the owner | unit |
| 026 | The owner's account is not locked on Alpine and the BSDs, whose sshd refuses the key of a locked account | unit |

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
| 008 | An admin can destroy a lab they do not own | e2e |
| 009 | Each lab operation is carried out for the lab's owner and for an admin, and refused to any other user, over RPC, REST and the web UI | integration |
| 010 | A lab's files, node secrets and host keys are given to its owner only, an admin refused, over every transport | integration |
| 011 | A user's lab list holds their own labs only; an admin gets another user's by asking for them, and a request with no login gets none | integration |
| 012 | The page and the progress stream of a web job are served to its creator and to an admin, and to no other user | integration |
| 013 | A node operation is refused to a user who does not own the lab, and an unknown lab or node is answered not found | integration |
| 014 | Only an admin's manifest may give a container host access: privileged mode or a host volume | integration |
| 015 | A request with no token is refused: auth required for every RPC method but login and validate, 401 over REST, and a page sends the browser to the login page | integration |
| 016 | The CLI refuses to connect to the nodes of a lab folder whose lab belongs to another user, before it asks the server, and takes a folder with only a manifest as the user's own | unit |
| 017 | A refusal for CPUs another lab holds names that lab to its owner and to an admin, and says only "another lab" to anyone else | unit |
| 018 | The server never reads a file a manifest names, such as a key file or a script: the client sends the content | integration |

## `account_keys`: a user's SSH keys are added only by that user

**Statement:** the SSH public keys on a user's account are valid OpenSSH keys, each held once, added only by that user, an admin included, and removed by that user or an admin.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | An admin cannot add an SSH key to another user's account, and the account's keys are unchanged | e2e |
| 002 | A user adds a key from a `.pub` file with `sherpa server user ssh-key add`, sees it listed with its fingerprint, and removes it by its `SHA256:` fingerprint (no test at this level yet) | e2e |
| 003 | Each supported key type is accepted: Ed25519, RSA, ECDSA P-256, P-384 and P-521 and the two security key types, with or without a comment or surrounding space | unit |
| 004 | An empty key, a key over more than one line, key data that is not a key of its type, a DSA key and an unknown type are each refused, with the reason | unit |
| 005 | A key already on the account is refused, even with a different comment, and the account's keys are unchanged | integration |
| 006 | A key is removed by the key itself, whatever its comment, or by its fingerprint, and a key that is not on the account is not found | integration |
| 007 | An admin removes a key from another user's account, over RPC, REST and the admin's page for the user | integration |
| 008 | A user who is not an admin cannot remove a key from another user's account, and it stays | integration |
| 009 | Of two key edits made from the same read of an account, the second is refused as a conflict and the first is kept | integration |
| 010 | The profile page refuses an invalid key with the reason, adds a valid one and removes one, showing the list each time | integration |

## `user_accounts`: an admin creates, lists and deletes user accounts

**Statement:** an admin creates, lists and deletes user accounts, each under a unique lowercase username that can be a login name on a node, and no account is deleted while it owns labs or is the one deleting.
**Relied on by:** the admin, the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | An admin creates a user with `sherpa server user create`, and the new user logs in (no test at this level yet) | e2e |
| 002 | A user created as an admin is one, and one created without is not, over RPC and REST | integration |
| 003 | A username is stored in lower case and trimmed, and the account is found and logs in under any case | integration |
| 004 | A username already taken, in any case, is refused | integration |
| 005 | A name that is not 3 to 32 of `a-z`, `0-9`, `_` and `-` starting with a letter or `_` is refused, naming the rule | unit |
| 006 | A reserved name is refused: Sherpa's own and privileged accounts, system accounts, the default users of guest images, and any name starting `systemd-` | unit |
| 007 | The database itself refuses a username that breaks the rule or is taken, whatever writes it | integration |
| 008 | A new account has no SSH keys, and a create request that carries keys is refused | integration |
| 009 | A create with a password that breaks the password rule is refused, and no account is made (no test yet) | integration |
| 010 | `sherpa server user list` shows each account with whether it is an admin and whether its device password is set (no test at this level yet) | e2e |
| 011 | A user is shown their own account details and an admin anyone's; any other user is refused, over RPC and REST (no test at this level yet) | integration |
| 012 | Account details never carry the password hash or the device password (no test yet) | integration |
| 013 | An admin deletes a user with `sherpa server user delete`, and that user can no longer log in (no test at this level yet) | e2e |
| 014 | Deleting your own account is refused, over RPC, REST and the web UI | integration |
| 015 | A user who owns labs is not deleted, and the refusal lists the labs, over RPC, REST and the web UI | integration |
| 016 | The database itself refuses to delete a user who owns labs | integration |
| 017 | Deleting a user that does not exist is answered not found | integration |
| 018 | With signup off, the server's default, the signup page and form answer 404 and no account is made | integration |
| 019 | With signup on, the signup page is served and the login page links to it | integration |
| 020 | Signing up makes an account that is not an admin and signs it in (no test yet) | integration |
| 021 | Signup refuses passwords that differ, a taken name, a name that breaks the rule and a weak password, each with its reason (no test yet) | integration |

## `builtin_admin`: the built-in `admin` account always exists and runs the server, not labs

**Statement:** the built-in `admin` account always exists and stays an admin, whatever is asked of the server or written to its database, and it cannot start lab work.
**Relied on by:** the admin. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | sherpad refuses to start when the `admin` account does not exist and it is given no password to create it, naming both ways to create it | integration |
| 002 | Given an admin password at startup, sherpad creates `admin` when it is missing, even when another admin exists, and leaves an existing `admin` and its password alone | integration |
| 003 | An admin password that breaks the password rule creates no `admin` account | integration |
| 004 | Deleting `admin` is refused, over RPC, REST and the web UI | integration |
| 005 | The database itself refuses to delete, rename or demote `admin`, or to create it without admin rights, by a single write or one that covers every user, and leaves any other admin alone | integration |
| 006 | The password and SSH keys of `admin` can be changed | integration |
| 007 | `admin` is refused up, resume and redeploy, over RPC, REST and the web UI, and told to use a personal account | integration |
| 008 | `admin` has no device password: setting one is refused | integration |

## `user_login`: a username and its password start a session

**Statement:** a user who gives their username and password gets a session and its token, on the CLI, the REST API and the web UI, and a wrong password and an unknown username are refused alike. Failed logins are limited per username and per source address (per /64 prefix for IPv6), the same on every path, and a login the limit refuses is answered as a wrong password is.
**Relied on by:** the user, scripts. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa login` with the right password saves a token, and `sherpa whoami` then names the user, whether they are an admin and when the token expires (no test yet) | e2e |
| 002 | `sherpa login` with a wrong password fails and saves no token (no test yet) | e2e |
| 003 | `sherpa login` takes the username and password from `SHERPA_USER_NAME` and `SHERPA_USER_PASSWORD` without a prompt (no test yet) | e2e |
| 004 | `sherpa whoami` with a token that is invalid or expired, or whose session has ended, exits non-zero and says to log in (no test yet) | e2e |
| 005 | A wrong password and an unknown username get the same refusal over RPC | integration |
| 006 | The REST login gives a token, the admin flag and an expiry 7 days out for the right password, and 401 with one message for a wrong password or an unknown username | integration |
| 007 | The web login form sets the session cookie and sends an admin to the users page and anyone else to the dashboard; a wrong password or an unknown username gets one message and no cookie | integration |
| 008 | The session cookie is `HttpOnly`, `Secure` and `SameSite=Strict`, for 7 days, or 30 with "remember me" | unit |
| 009 | An unknown username's password is still checked, against a real hash, so the refusal takes as long as a wrong password's | unit |
| 010 | A login request without a username or a password is refused as invalid | integration |
| 011 | The login page is served to anyone, sends on a browser that is already signed in, and shows the notice for a session that is needed, has expired or was logged out | integration |
| 012 | The CLI keeps the token in a file only the user can read, and a command run with none saved says to run `sherpa login` | unit |
| 013 | A token is checked over REST as over RPC: a valid one names its user, admin flag and expiry, and any other is answered not valid, not an error | integration |
| 014 | An RPC login with the right password gives a token, the username, the admin flag and the expiry, and the token names a session that lasts as long | integration |
| 015 | The CLI reads the signed-in user's name from the saved token, and says to run `sherpa login` when the token is not one | unit |
| 016 | After 5 failed logins for a username within 15 minutes, its right password is refused with the answer a wrong password gets, over RPC, REST and the web login form, whichever paths the failures came by | integration |
| 017 | After 20 failed logins from an address within 15 minutes, a username that has not failed is refused from it with the answer a wrong password gets, over RPC, REST and the web login form | integration |
| 018 | The built-in `admin` is locked after 5 failed logins, like any other account | integration |
| 019 | A locked username is refused for 15 minutes from when it was locked, and let in after | unit |
| 020 | A locked address is refused for 15 minutes from when it was locked, and let in after | unit |
| 021 | Failures 15 minutes or more after a username's first counted failure start a new count | unit |
| 022 | A successful login clears its username's count | unit |
| 023 | A successful login does not count toward its address's limit | unit |
| 024 | A username is counted as accounts are looked up, in any case and with surrounding space | unit |
| 025 | An IPv4 address is counted the same whether the connection gives it as IPv4 or as IPv4-mapped IPv6 | unit |
| 026 | Logins made at the same moment check no more passwords than the limit allows: an attempt still being checked counts | unit |
| 027 | An attempt that ends without an answer, such as when the database cannot be read, is not counted as a failure | unit |
| 028 | However many usernames and addresses fail, counts are kept for no more than 10,000 of each, so a flood cannot grow the server's memory without limit | unit |
| 029 | When the counts are full, a lock is kept over a count that has not reached its limit | unit |
| 030 | A lockout is logged once, at `warn`, with the username and the address | unit |
| 031 | IPv6 addresses are counted by their /64 prefix: two addresses in one /64 share one count and are locked together, an address in another /64 is not, and the lock is logged under the prefix, such as `2001:db8:1:2::/64` | unit |

## `user_sessions`: a token works only while its session lasts

**Statement:** a token works only while its session lasts, which is until it expires or its user logs out, changes their password or is deleted; after that every transport refuses it, and no other session is touched.
**Relied on by:** the user, the admin. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa logout` ends the session on the server and removes the saved token, and a copy of the token is then refused (no test yet) | e2e |
| 002 | A logout over RPC ends that session only: the user's other sessions still work | integration |
| 003 | A logout over REST ends the session of the bearer token | integration |
| 004 | A web logout ends the cookie's session on the server, clears the cookie and goes to the login page | integration |
| 005 | Logging out of a session that has already ended is not an error, and the CLI reports it as already ended | unit |
| 006 | When the server cannot end the session, `sherpa logout` still removes the saved token and warns that the session may still work | unit |
| 007 | A session past its expiry is refused, before the sweep has deleted it | integration |
| 008 | A token that is expired, signed with another secret, malformed, or names no session is refused | unit |
| 009 | A token whose session has ended is refused on every transport: auth required over RPC, 401 over REST, and a web page clears the cookie and goes to the login page | integration |
| 010 | A browser with no session cookie is sent to the login page, with no cookie set | integration |
| 011 | The CLI answers a refused token by saying to run `sherpa login` | unit |
| 012 | Changing your own password ends your other sessions and keeps the one it was changed from | integration |
| 013 | An admin's reset of a user's password ends every session of that user and none of the admin's | integration |
| 014 | A password change that is refused ends no session | integration |
| 015 | Deleting a user ends their sessions, on every transport | integration |
| 016 | A user created again under a deleted user's name is not let in by the old account's tokens | integration |
| 017 | A user's admin rights are read on every request, so a token issued before they changed has the new rights at once, on every transport | integration |
| 018 | The sweep deletes the sessions that have expired and no other, when sherpad starts and at each interval, and stops when sherpad shuts down | integration |
| 019 | A REST request takes the bearer token before the cookie, and the cookie when the bearer token is not valid | unit |
| 020 | Of two session cookies in one request, a valid one is used whichever comes first | unit |
| 021 | A refused session cookie is logged with its reason and never with its value | unit |
| 022 | A job started in the web UI runs as its requester only while their session lasts | integration |
| 023 | sherpad generates its token secret once, in a file only it can read, and loads the same one on the next start | unit |
| 024 | sherpad refuses to start on a token secret of the wrong length, names the file and leaves it as it is | unit |
| 025 | A page of another origin is never granted cross-origin access to the API | integration |

## `user_passwords`: a login password is changed by its user, or reset by an admin

**Statement:** a login password meets the password rule, is stored only as a salted hash, and is changed by its user, who must give the current one, or reset by an admin.
**Relied on by:** the user, the admin. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa server user passwd` with the current password changes it: the old password no longer logs in and the new one does (no test at this level yet) | e2e |
| 002 | Changing your own password without the current one, or with a wrong one, is refused over RPC, for an admin too, and the password is unchanged | integration |
| 003 | The same over REST | integration |
| 004 | The profile page refuses a wrong current password and new passwords that differ, and changes the password with the right one | integration |
| 005 | An admin resets another user's password without knowing it, over RPC, REST and the admin's page for the user | integration |
| 006 | A user who is not an admin cannot change another user's password | integration |
| 007 | The admin's page for their own account refuses to reset their password and points to the profile page (no test yet) | integration |
| 008 | A password needs 8 characters, an uppercase letter, a lowercase letter and one of `!@#$%^&*_+-=`, and a refusal lists every part it lacks | unit |
| 009 | A new password that breaks the rule is refused at change and at reset, and the old one still logs in | integration |
| 010 | A password is stored as an Argon2id hash with a salt of its own, so the same password hashes differently each time, and only the right password verifies | unit |
| 011 | A hash stored by an earlier release still verifies | unit |
| 012 | A stored hash that cannot be checked is an error, not a wrong password | unit |
| 013 | A password change keeps an edit of the user's other details made at the same time | integration |

## `device_password`: a user's device password is theirs alone

**Statement:** a user's device password is set only by that user, with their login password, meets one rule that every node model takes, and is never shown again.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa server user device-password` sets it, and `sherpa server user info` then shows it as set (no test at this level yet) | e2e |
| 002 | Nobody sets another user's device password, an admin included, over RPC and REST, and nothing is stored | integration |
| 003 | A wrong login password is refused and nothing is stored | integration |
| 004 | A device password that is the user's login password is refused | integration |
| 005 | A device password has 12 to 20 characters | unit |
| 006 | It has only letters, digits and `! @ _ - + .`, starting with a letter or a digit | unit |
| 007 | It has a lowercase letter, an uppercase letter, a digit and one of the punctuation characters | unit |
| 008 | It holds neither the username nor `cisco`, in any case, and no character three times in a row | unit |
| 009 | Setting it again replaces it | integration |
| 010 | Account details and the user list carry only whether it is set | integration |
| 011 | It is stored sealed with the database key for its user's row: it opens only for that row with that key, and the database refuses a value that is not sealed | integration |
| 012 | The profile form sets the signed-in user's own with their login password and shows it as set; a form that names a user, a wrong login password and passwords that differ are refused | integration |
| 013 | The CLI asks for the login password, then the new one twice, takes no flag for either, and stops when the two differ, one is empty or input cannot be read | unit |
| 014 | A device password, in a request or opened from the database, shows nothing of itself in debug output | unit |

Each refusal in cases 005 to 008 names the part of the rule that was broken and never repeats the password; their tests check both.

## `admin_only`: server administration is carried out only for an admin

**Statement:** managing users, images, the clean of a lost lab and the database backup, and every admin page, is carried out only for an admin.
**Relied on by:** the admin, the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A user who is not an admin is refused `sherpa server user create`, `list` and `delete`, and nothing changes (no test yet) | e2e |
| 002 | Every admin RPC method is refused to a user who is not an admin, as access denied | integration |
| 003 | Every admin REST operation is refused to a user who is not an admin with 403, and to no login with 401, and nothing changes | integration |
| 004 | Every admin page is served to an admin, is a 403 page for any other user, and sends a browser with no session to the login page | integration |

The image methods, routes and pages in these lists are `image_access_002` and `image_access_003` in [images.md](images.md).
