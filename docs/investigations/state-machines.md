# Implicit state machines: invalid states and what to make explicit (#474)

Issue #474 asked for every implicit state machine in Sherpa, the invalid states each can
reach, and a ranked list of which to make explicit. How each machine works today, with its
diagram, is in [`../architecture/STATE_MACHINES.md`](../architecture/STATE_MACHINES.md). The
machine numbers below (M1 to M32) are the section numbers there. This note does not repeat
those sections.

## Method

- **Search.** Every crate was searched for the signs listed in #474:
  - fields that must agree;
  - bools or `Option`s that stand for a stage;
  - states held in strings;
  - a state set in several places with no check of the state it came from;
  - repeated "X is set and Y is not" checks.

  The crates were server, client, db, libvirt, container, network, shared, topology, template
  and iso.
- **Verification.** Every sequence below was traced in the code at the commit that added this
  note. Nothing was run against a live server.
- **Inference.** A step that depends on libvirt, Docker, the kernel or timing, and not on
  Sherpa code, is marked *(inferred)*.
- **Rejected candidates.** These have a lifecycle but no invalid state:
  - closed enums used only inside one function: `StopPlan`, `ContainerResume`, `VmReadiness`,
    `KeyMismatch`, `PromptScope`;
  - the ISO writer, which writes a `.partial` file and renames it (`crates/iso/src/writer.rs`);
  - `RpcClient::close(self)`, where the compiler already prevents reuse;
  - the Talos, Vault and Keycloak data types themselves;
  - the per-image lifecycle lock (`image_lifecycle.rs`).
- **Not analysed.** The SSH client's raw-mode and `~.` escape handling are in the external
  `jato` crate.

## Invalid states reachable today

### M1 and M2. Node lifecycle and lab status

1. **Nodes and the lab show `Stopped` while `up` is still building them.**
   - Sequence:
     1. `up` phase 3 inserts the node rows as `Unknown` (`crates/db/src/node/create.rs:55`).
     2. The scanner runs; its default interval is 30 s (`crates/shared/src/data/config.rs:180`).
        It takes no lease.
     3. VMs are only defined in phase 11, and containers are only started in phase 13. Until
        then there is no domain or container for a node, so `detect_node_state` returns
        `Stopped` (`crates/server/src/services/scanner.rs:320`, `:334`).
     4. The scanner writes `Stopped` for the node (`scanner.rs:172`). `LabState::derive`
        then gives `Stopped`, which is written for the lab (`scanner.rs:219`).
   - Result: the web UI's `{% else %}` arm offers Start for a lab that is being created.
   - A `resume` started then shares the read lease with `up`
     (`crates/server/src/services/lab_operations.rs:134`), and starts any domain that already
     exists.
   - Any `up` that runs longer than one scan interval reaches this.
2. **`Failed` lasts at most one scan.**
   - Sequence: `rebuild` fails, and `mark_failed` writes `Failed`
     (`crates/server/src/services/redeploy/node.rs:83-86`, `:180`).
   - On the next scan, the node is written as:
     - `Stopped`, if the domain or container is gone (`scanner.rs:320`, `:334`);
     - `Running`, if it was already recreated.
   - No code path other than redeploy produces `Failed`, so `LabState::Failed` is almost never
     stored.
3. **`Running` for a node that is not ready.** There are three ways:
   - `resume` writes `Running` as soon as `domain.create()`, `resume()` or a container start
     returns (`crates/server/src/services/resume.rs:136-141`). The readiness wait comes after,
     and a timeout changes only the message.
   - The scanner writes `Running` for any domain libvirt reports as running, whether or not
     the guest has finished booting.
   - Redeploy's doc comment says a VM that times out is "left `Starting`"
     (`redeploy/vm.rs:199-202`). That holds only until the next scan.
4. **`Running` for a crashed VM after `resume`.**
   - Sequence: a domain is in libvirt state CRASHED, SHUTDOWN, BLOCKED or PMSUSPENDED. `start_vm`
     falls into the "Already running or other state" branch and reports success
     (`resume.rs:539-545`).
   - The node is written as `Running` (`resume.rs:138`). The next scan writes `Unknown`
     (`scanner.rs:319`).
5. **`down` races `up` or `resume`.** `shutdown_lab_nodes` takes no lease (`down.rs:98`). Two
   sequences:
   - `down` runs while `up` is in phases 4 to 12. `down` reports the containers as not
     running. `up` then starts them in phase 13 and writes `Running`, so the lab is up after
     a successful `down`.
   - `up` probes VM A as ready, then `down` stops A and writes `Stopped` (`down.rs:165`), then
     `up` writes `Running` for A (`up/readiness.rs:416`). The ordering is *(inferred)*; the
     unguarded writes are verified.
6. **A stale scanner snapshot overwrites a newer write.**
   - `scan_cycle` queries libvirt and Docker once (`scanner.rs:59-60`), then walks every lab.
     It may read SSH host keys on the way, with a timeout per node (`record_host_keys`,
     `scanner.rs:235`).
   - A `down` that writes `Stopped` after the snapshot is overwritten with `Running`
     (`scanner.rs:172`). The next cycle corrects it.
7. **`Unknown` forever.**
   - With `scanner.enabled = false` (`crates/server/src/daemon/server.rs:240`), nothing ever
     writes `lab.status`. Every lab stays at its creation default, `Unknown`
     (`crates/db/src/lab/create.rs:113`).
   - Labs with no nodes are skipped by the scanner (`scanner.rs:126`), so `LabState::Empty` is
     never stored either.
8. **One unknown state string breaks listing.**
   - `node.state` and `lab.status` are TEXT with no CHECK (`crates/db/sql/schema/node.sql:11`,
     `lab.sql:16`). They are decoded with serde.
   - A value written by a newer build fails the decode of the whole query, so `list_labs` fails
     for everyone. The downgrade case is *(inferred)*; the strict decode is verified for
     `node_image` by a test in `crates/db/src/schema/apply.rs`.

### M3. Lab operation leases

1. **Two `up`s with the same lab ID: the second one's rollback deletes the first one's lab.**
   - The lab ID is a hash of the username and the lab name
     (`crates/shared/src/util/random.rs:21-34`). Running `sherpa up` twice for the same
     manifest, or starting it from the CLI and the web UI, gives the same ID.
   - Sequence:
     1. Both calls take shared read leases (`crates/server/src/services/up/lab.rs:145`).
     2. Both pass `find_lab` in `set_up` (`up/lab.rs:283`) and `ensure_no_earlier_attempt`
        (`:314`) before either creates the lab row.
     3. The second `db::create_lab` fails on the unique `lab_id`. The subnets are not taken,
        so `reserve_lab_networks` returns the error (`up/networks.rs:243-245`).
     4. That error happens in phase 3, so `roll_back_failed_up` runs (`up/lab.rs:218-219`).
     5. It calls `clean_lab_resources(lab_id)` (`up/lab.rs:828`), which removes every
        container, domain, disk, network and interface named for the lab ID. It then removes
        the first run's lab row and directory (`crates/server/src/services/clean.rs:67-76`).
   - The RPC pre-check (`crates/server/src/api/websocket/rpc/labs.rs:165`) has the same window.
2. **`down` and the scanner do not take part in the leases** (see M1.1 and M1.5).
3. **`LAB_LEASES` never shrinks.** Entries are added per lab ID (`lab_operations.rs:125-130`)
   and never removed, so memory grows by one entry for each lab the server has ever seen.
   This is a small leak.

### M4. Runtime state mappings

1. **A DEAD container is shown as Startable, but cannot be started.**
   - The scanner maps DEAD to `Stopped` (`scanner.rs:327`), and the UI offers Start.
   - `container_resume_step` returns `Cannot` for DEAD (`resume.rs:587-589`), so Start fails.
2. **A missing domain or container looks like a stopped one** (`scanner.rs:320`, `:334`). This
   is also the mechanism behind M1.1 and M7.3.

### M5. Readiness and host keys

1. **No host key for a node that failed redeploy.**
   - `redeploy_node` clears the key (`redeploy/node.rs:76`) before the rebuild. If the rebuild
     fails, the node is `Failed` with no key.
   - The next SSH file refresh writes `known_hosts` without it.
   - The scanner reads a key again only for a running VM (`scanner.rs:274-288`).
2. **The scanner rewrites a lab's SSH files with no coordination.**
   - It rewrites them after it records a key (`scanner.rs:257-267`). It takes no lease and no
     redeploy lock, while `up` (`up/lab.rs:687`) and redeploy (`redeploy/node.rs:89`)
     rewrite the same files.
   - Both sides build the files from the database, so the last writer wins with fresh data.
     Whether they can interleave inside one file write is *(inferred)*: `create_file` is a
     plain write.

### M6. `up` phases and rollback

1. **`success: true` for a lab that is only partly up.**
   - Every `UpError` is constructed with `is_critical: false`:
     - `up/lab.rs:455`;
     - `up/services.rs:167`;
     - `up/readiness.rs:539`, `:564`.
   - So `success = critical_errors == 0` (`up/lab.rs:745-747`) is true whenever `up` returns
     `Ok`. That includes these cases, each reported only as a warning:
     - a node that never became ready;
     - a container that exited;
     - a Talos cluster that did not bootstrap;
     - a Vault that failed to initialise.
   - The client's "only partly created" error (`crates/client/src/cmd/up.rs:167-173`) is
     therefore unreachable.
2. **A stranded lab after a failed rollback.**
   - Sequence:
     1. `up` fails after phase 3.
     2. `clean_lab_resources` hits any resource error.
     3. `cleanup_lab_metadata` keeps the lab row and directory
        (`crates/server/src/services/destroy.rs:353-361`).
     4. `roll_back_failed_up` only logs the result.
   - A retry of `up` is refused because the row exists (`up/lab.rs:283`). The user has to know
     to run `destroy`.
3. **Daemon killed during `up`.** Nothing rolls back, and nothing at startup looks for
   half-built labs. The only lab scan at startup is the scanner (`scanner.rs:81`). The lab row,
   node rows, domains, containers and files all stay. Recovery is the same as M6.2.
4. **The node row and the response disagree.**
   - Skipped nodes and containers that exit at once are reported in `NodeInfo` as `Unknown` or
     `Stopped` (`up/readiness.rs:207`, `:316`, `:327`). The row is not written, and stays
     `Unknown` until a scan.
5. **CPU pins are checked before they are reserved.**
   - `check_cpu_pins_on_server` runs in planning (`up/plan.rs:129`). The pins are only stored
     in phase 3, and the column has no uniqueness constraint (`node.sql:16`).
   - Two `up`s of different labs can both pass the check with overlapping pins. They hold
     different leases, so nothing serializes them. The timing is *(inferred)*.

### M7. Destroy and clean

1. **A lab directory with no row cannot be removed through the API.**
   - Sequence:
     1. A destroy deletes the row (`destroy.rs:368`), then `fs::remove_dir_all` fails
        (`destroy.rs:391-401`).
     2. `destroy` now fails at `require_lab_access`, because the lab no longer exists
        (`destroy.rs:60`).
     3. `clean`, which is documented to handle labs that are not in the database
        (`clean.rs:19-24`), calls `delete_lab`. That fails with "Lab with lab_id not found"
        (`crates/db/src/lab/delete.rs:38`).
     4. `cleanup_lab_metadata` records a `database` error and returns before it reaches the
        directory (`destroy.rs:374-383`).
   - The directory stays until someone removes it by hand.
   - A later `up` of the same lab (same ID, see M3.1) creates its files in that directory.
     Keycloak's `realm_credentials` returns the old saved users and address unchanged
     (`crates/server/src/services/keycloak/lab.rs:80-82`).
2. **A second concurrent destroy reports failure for a lab that is gone.**
   - The second destroy checks access before it takes the lease (`destroy.rs:60`, `:74`), and
     does not check again after.
   - Once it gets the lease, its `delete_lab` fails (`crates/db/src/lab/delete.rs:38`), so `success` is false
     (`destroy.rs:110`).
3. **A half-destroyed lab looks `Stopped`.**
   - A destroy with resource errors keeps the lab row. There is no "destroying" or
     "destroy failed" state.
   - The scanner maps the missing domains and containers to `Stopped` (M4.2), so the UI offers
     Start. `resume` then fails per node with "Domain not found"
     (`resume.rs:500-506`).

### M8. Down and resume

1. **Unikernels are not re-attached after a cold boot.**
   - Only `NodeKind::VirtualMachine` goes into `cold_booted_vms` (`resume.rs:143-145`).
   - Unikernels can have P2p interfaces.
   - So after `down` and `resume`, a unikernel has a new tap with no redirect, and its peer's
     redirect points at the old ifindex. That redirect behaviour is *(inferred)*.
2. **The state is in a display string.**
   - Cold boot is detected by `result.message == "Started"` (`resume.rs:143`, `:147`).
   - Rewording the message in `start_vm` (`resume.rs:517`) or `start_container_node`
     (`resume.rs:618`) silently turns off the eBPF re-attach and the readiness wait.
3. **A Tailscale failure aborts the resume before any node starts.**
   - `resume_tailscale(...)` runs first, with `?` (`resume.rs:80-81`).
   - If the gateway started but enrolment failed, the gateway is running and every node is
     still down.

### M9. Redeploy

1. **`Failed` for a container that is running.**
   - `redeploy_container` writes `Running` (`redeploy/container.rs:149`), then restores Vault
     and Keycloak.
   - If that restore fails, `rebuild` returns an error and `mark_failed` writes `Failed`
     (`redeploy/node.rs:83-86`) for a running container.
2. **Redeploy fails after teardown when a peer is down.**
   - `reattach_p2p_redirects` calls `get_ifindex(...)?` on both ends of every link
     (`redeploy/links.rs:53-58`).
   - If a peer VM is shut off, its tap does not exist. The rebuilt node is then marked
     `Failed`, although it is up.
   - The peer's own redirect still points at the old ifindex.

### M10 to M13. Provisioning stages

1. **A Talos cluster that did not bootstrap is never retried.**
   - `bring_up_clusters` is called only from `up` (`up/services.rs:55`). The failure is a
     non-critical error (M6.1).
   - Redeploy refuses control-plane nodes (`talos/lab.rs:88-99`).
   - The only recovery is destroy and up.
2. **A Vault can be initialised with no saved key.**
   - `client.init()` (`vault/lab.rs:103`) and `save_credentials` (`vault/lab.rs:117`) are two
     steps. If the save fails, or the process dies between them, Vault is initialised and
     sealed with no key on disk.
   - Every later run fails with "initialised outside Sherpa" (`vault/lab.rs:96-101`).
   - Only a redeploy of the Vault node, which brings empty storage, recovers it.
3. **Keycloak users are saved before they exist.**
   - `realm_credentials` writes the users file before the realm is created
     (`keycloak/lab.rs:73-97`). If realm creation fails, the file names users Keycloak does not
     have.
   - Nothing re-runs the realm setup except a redeploy of the Keycloak node.
4. **A Tailscale volume left behind blocks the next gateway.**
   - `create_gateway` creates the volume, then the container
     (`crates/container/src/tailscale.rs:127-176`). It refuses when the volume already exists
     (`:135-141`).
   - If the container create fails and the rollback's cleanup also fails, the next `up` stops
     at "run lab cleanup before retrying".

### M14. Sessions and tokens

**Resolved by #501 (0.5.21).** Sessions are rows in a `session` table, and the states and their
transitions are explicit in `crates/server/src/auth/session.rs`. How it works now is in
[`SERVER.md`](../architecture/SERVER.md#sessions-and-identity) and
[`STATE_MACHINES.md` §14](../architecture/STATE_MACHINES.md#14-sessions-and-tokens-server). Each
item below is kept as found, with what closed it:

- M14.1: logout (web, REST, RPC and `sherpa logout`) deletes the session; a password change
  deletes the user's other sessions, or all of them on an admin reset. Tests:
  `auth::session` tests `a_token_used_after_logout_is_refused` and
  `a_token_used_after_a_password_change_is_refused`, and the integration tests
  `rpc_logout_ends_only_that_session`, `rest_logout_ends_the_bearer_session`,
  `web_logout_ends_the_cookie_session_on_the_server`,
  `changing_your_password_ends_your_other_sessions` and
  `an_admin_password_reset_ends_every_session_of_the_user`.
- M14.2: sessions belong to the user's id and are deleted with the user (`ON DELETE CASCADE`).
  Tests: `a_token_of_a_deleted_user_does_not_work_for_a_new_user_of_the_same_name` and the
  integration test `a_recreated_user_does_not_inherit_the_old_accounts_tokens`.
- M14.3: a web job presents its requester's session again when it starts
  (`Job::resolve_requester`). Tests in `daemon::state`: refused after logout and after deletion,
  runs with a demoted user's current rights.
- M14.4: a secret file of the wrong length stops startup with an error naming the file. Test:
  `a_secret_of_the_wrong_length_refuses_startup_and_is_kept`.
- M14.5: `update_user` is compare-and-set on `updated_at` (`db::UserChanged`, a `Conflict` to
  callers), and a password change writes only the hash. Tests:
  `test_update_from_a_stale_read_is_refused`, `a_password_change_keeps_concurrent_ssh_key_edits`
  and `a_key_edit_from_a_stale_read_is_a_conflict_and_keeps_the_other_edit`.

1. **A token outlives logout and password changes.**
   - `logout_handler` only clears the cookie (`crates/server/src/api/handlers/auth.rs:440-452`).
   - `change_password` only rewrites the hash
     (`crates/server/src/services/user_password.rs:61-65`).
   - A copied token stays valid until `exp`: 7 days (`crates/shared/src/konst.rs:510`), or
     longer for a web "remember me" login.
2. **A deleted user's token works for a new account with the same name.**
   - `resolve_user` looks users up by `claims.sub`, which is the username
     (`crates/server/src/auth/middleware.rs:17`, `:45-48`).
   - Sequence:
     1. An admin deletes `alice` (`crates/server/src/services/user_delete.rs:60`).
     2. A new `alice` is created, through signup (`crates/server/src/api/handlers/auth.rs:310`) or by an admin
        (`crates/server/src/api/handlers/users_api.rs:34`).
     3. The old token authenticates as the new account.
3. **A queued web job runs with its creator's old rights.**
   - A `Job` stores the requester's `AuthContext` (`crates/server/src/daemon/state.rs:25-32`).
   - It runs with that context when the stream is opened (`labs_web.rs:765`, `:773`), even if
     the user was demoted or deleted in between.
4. **Every session ends at once when the JWT secret file is the wrong length.**
   - `load_or_generate_secret` regenerates the secret and only warns
     (`crates/server/src/auth/jwt.rs:45-51`).
5. **Concurrent edits to a user lose updates.**
   - `update_user` writes every column (`crates/db/sql/queries/user/update.sql`).
   - Password change and SSH-key edits each read the row, change one field and write it all
     back. Of two concurrent edits, the second overwrites the first. The timing is *(inferred)*.

### M15 and M16. Web jobs and streams

1. **A job that is never streamed stays in memory until restart.**
   - Only the stream handler removes jobs (`labs_web.rs:745`). There is no expiry.
   - A create job holds the whole `UpRequest`.
   - A destroy job that is never streamed never runs.
2. **A running job cannot be re-attached.**
   - The stream handler removes the job when it starts it. A reload then shows "Job not found
     or already completed" (`labs_web.rs:678`), while the work goes on.
3. **A client that disconnects loses the result.**
   - WebSocket RPCs are spawned (`crates/server/src/api/websocket/handler.rs:132`). Progress
     and the final response are dropped when the client has gone
     (`crates/server/src/api/websocket/rpc/dispatch.rs:296-298`, `:307-308`).
   - Nothing records the outcome for the client to fetch later.

### M17 to M20. Daemon, TLS and init

1. **Two daemons.**
   - Two `sherpad start` commands both pass `verify_not_running`
     (`crates/server/src/daemon/manager.rs:30`) before either writes the PID file.
   - `write_pid` is a plain `fs::write` (`crates/server/src/daemon/pidfile.rs:32-36`), so the
     last writer wins. The other daemon is then untracked, or fails to bind *(inferred)*.
2. **A PID reused by another process is treated as the daemon.**
   - Liveness is `kill(pid, 0)` (`pidfile.rs:67-70`). `stop_daemon` would send SIGTERM to that
     process (`manager.rs:115-178`).
   - Reaching this requires PID reuse *(inferred)*.
3. **An expired or wrong-SAN server certificate is served.**
   - `ensure_certificates` checks only that the files exist
     (`crates/server/src/tls/certificate_manager.rs:45-50`).
   - The generated certificate lasts `cert_validity_days`, 365 by default
     (`crates/shared/src/data/config.rs:102`). After that, clients fail the handshake until
     someone deletes the files.
4. **A user-supplied certificate is overwritten.**
   - If only one of the two files exists and `auto_generate_cert` is on, both are regenerated
     (`certificate_manager.rs:52-55`).
5. **A lab CA certificate without its key.**
   - `create_lab_ca` writes the certificate, then the key (`crates/server/src/tls/lab_ca.rs:72-83`).
   - A crash between the two writes leaves a lab that `load_lab_ca` cannot load, so redeploy
     fails. Reaching this requires a crash *(inferred)*.
6. **A partial `sherpad init`.** No step is undone (`crates/server/src/init.rs:37-138`). Re-running
   it is the recovery, and depends on the libvirt network and pool creates being idempotent
   *(inferred)*.

### M21 to M24. Client

1. **Contradictory TLS settings are accepted without a word.**
   - `insecure` together with `ca_cert_path`: the CA is ignored
     (`crates/shared/src/tls/config.rs:70`).
   - `ca_cert_path` together with a pinned certificate or `validate_certs`: both are ignored
     (`config.rs:78-93`).
   - A `ws://` URL with any TLS setting: all are ignored (`crates/client/src/ws_client/client.rs:82-95`).
     `--insecure` still prints that validation is disabled (`crates/client/src/cmd/cli.rs:55-60`).
   - A set `url` overrides `tls.enabled` (`crates/shared/src/util/config.rs:49-51`).
2. **A stale pin has no recovery in the connection flow.**
   - When the server's certificate changes, the pinned one is still chosen (`crates/shared/src/tls/config.rs:95-103`)
     and the handshake fails.
   - `unknown_issuer` is `None` on that path, so trust on first use is not offered. The user has
     to know to run `sherpa cert trust`.
3. **One token for every server.**
   - `~/.sherpa/token` is not keyed by server (`crates/client/src/token.rs:1-90`).
   - Talking to a second server gives "Your login has expired or is invalid"
     (`crates/client/src/ws_client/messages.rs:72-74`).
4. **The lab runs on the server with no local files.**
   - Answering "keep running" to Ctrl-C during `up` returns before the files are saved
     (`crates/client/src/cmd/up.rs:131`). Provisioning does not run either.
   - `destroy` and `down` have no Ctrl-C handling (`crates/client/src/cmd/destroy.rs:95-98`).
5. **Nodes that are not ready when `up` returns are never provisioned.**
   - Readiness is a snapshot: `ssh_port.is_some()` at the end of `up` (`up.rs:183`).
   - A provisioner that fails part way leaves the node half-provisioned with no record
     (`crates/client/src/provision/run.rs:154-166`).
   - `sherpa provision` re-runs everything on every node.

### M25 and M26. P2p links and impairment

1. **One failed link stops every later re-attach in a resume.**
   - `reattach_p2p_ebpf_for_nodes` uses `?` per link (`resume.rs:661-679`).
   - Sequence: VM A has links to B and C. B is shut off. Resume A alone. The B link fails at
     `get_ifindex(tap_b)`, and the A–C link is never re-attached.
   - The error is only logged (`resume.rs:152-165`), and A is `Running`.
   - Which link comes first depends on `list_links_by_lab` order.
2. **Container P2p veths are not recreated by resume.**
   - Veths come from `create_p2p_interfaces`, which only `up` and `redeploy` call.
   - A container that exited, and is started again by `resume` (`resume.rs:616-620`), gets a
     new network namespace without its veths *(inferred: a container's veths last only as long
     as its namespace, `crates/network/src/tap.rs:71-74`)*.
   - `down` pauses containers and does not trigger this.
3. **A failed netem removal is recorded as clear.**
   - `set_netem` treats every `remove_netem` error as "nothing to remove"
     (`crates/server/src/services/impairment.rs:172-180`).
   - The database is then set to clear (`impairment.rs:83-109`) while the kernel may keep the
     qdisc.
   - Re-attach skips clear directions (`crates/network/src/tc.rs:192-204`), so it is never
     cleaned up.
4. **The kernel is changed before the database.**
   - If `update_link_impairments` fails (`impairment.rs:102-109`), the kernel holds the new
     value and the database the old one. The next resume or redeploy puts the old value back.

### M27. Bridges and networks

1. **An existing Docker network with different settings is accepted.**
   - `create_docker_bridge_network` treats HTTP 409 as success
     (`crates/container/src/network/create.rs:62-65`) and does not compare the network's
     subnet or bridge.
2. **Orphaned bridges after a failed rollback.**
   - Between `network::create_bridge` and `db::create_bridge` (`up/networks.rs:369-387`) there
     is a Linux bridge with no row.
   - The rollback removes it by name (`destroy.rs:783-820`). It is left only when that cleanup
     also fails.

### M28. Node images

1. **A tracked image with no file, or a truncated one.**
   - `import_disk_image` upserts the row (`crates/server/src/services/import.rs:164`) before
     copying (`:187-201`). A failed copy leaves the row.
   - `copy_file` writes straight to the destination
     (`crates/shared/src/util/file_system.rs:110-125`), so a copy that fails part way leaves a
     truncated file.
   - The next import sees the file and skips the copy (`import.rs:187-191`), so the truncated
     image is then used.
2. **A model with no default.** There are three ways:
   - Re-import the current default version with `default = false`. `make_default` becomes
     false (`import.rs:52-63`), and the upsert replaces the `default` column
     (`crates/db/src/node_image/create.rs:61-68`).
   - Delete the default version. No other version is promoted
     (`crates/server/src/services/delete.rs`).
   - Download the first version of a model with `default = false`. `download_image` uses
     `request.default` directly (`import.rs:876`), unlike import and pull.

   The web form's "keep one default" check (`images_web.rs:344-368`) covers none of these.
3. **A scan can re-add an image while it is being deleted.**
   - Delete removes the row, then the files. `scan_images` takes no lock.
   - A scan between the two steps re-imports the files *(inferred, timing)*.

### M29 and M30. ZTP settings and `ztp_config`

1. **`ztp_enable = false` with `ztp_method = Tftp` or `Http`.**
   - Sequence: an admin unticks "enable" in the image form and leaves the method
     (`crates/server/src/api/handlers/images_web.rs:220-226`). The validation does not look at
     ZTP (`:373-379`).
   - On `up`, `generate_vm_ztp` skips all media (`crates/server/src/services/node_ops.rs:1015-1019`).
     The `ZtpRecord` still carries the method (`node_ops.rs:1046`).
   - dnsmasq then sends DHCP option 67 for a config file that was never written
     (`crates/template/templates/dnsmasq/config.jinja:48-52`).
   - A `ztp_config` in the manifest is dropped without a message: the custom-config check and
     the take (`node_ops.rs:1095-1096`) only run when ZTP is enabled.
2. **A custom config is accepted and then discarded.**
   - With `method = Volume` or `Ipxe`, `check_custom_ztp_supported` allows a custom config
     (`node_ops.rs:1060-1080`).
   - `take_custom_ztp_config` takes it and reports "Using custom ZTP config"
     (`node_ops.rs:350-363`, `:1096`).
   - The match arm then only prints "not applicable" or "not yet implemented"
     (`node_ops.rs:1131-1148`).
3. **A method the model does not support fails partway through `up`.**
   - The generators reject the combination with `bail!` deep inside `up` phase 8
     (`node_ops.rs:1880` and similar), after phases 3 to 7 created resources. The rollback then
     removes them.
   - Nothing checks the combination when the image is saved.
4. **Re-import overwrites admin edits.** The import and scan paths write the code defaults over
   every column of an existing row, ZTP included (`import.rs:40-70`, `crates/db/src/node_image/create.rs:61-68`).
5. **The ZTP credentials are never used.**
   - `ztp_username`, `ztp_password` and `ztp_password_auth` are editable in the form, but only
     the table display reads them (`crates/shared/src/util/table.rs:456-462`).
6. **A manifest path decoded as base64.**
   - A manifest that reaches the server without the client's `resolve_ztp_configs`
     (`crates/client/src/cmd/up.rs:195-227`) still holds a path in `ztp_config`. This happens,
     for example, when it is pasted into the web create form.
   - `decode_ztp_config` then fails with "Failed to decode ztp_config"
     (`crates/server/src/services/up/ztp.rs:458-463`).
   - `check_client_read_files` covers the other path fields but not this one
     (`crates/topology/src/validate/client_files.rs`).
   - A path that happens to be valid base64 would decode to garbage *(inferred)*.

### M31 and M32

- M31: see M6.5. Ports and pins stay reserved while a lab is down. That is by design.
- M32: no invalid state found. The schema version is applied atomically and refuses newer
  databases.

## Ranking: what to make explicit

The ranking weighs how easily a user reaches each state against what it costs: lost labs,
silently broken dataplanes, security, or confusion. The form named for each machine is a
direction for the fix, not a design.

1. **Node lifecycle and its coordination with the scanner (M1, M2, M4, M8.2).**
   - **Form:** a transition function over `NodeState`, with events (`Created`, `Probed`,
     `Observed(runtime)`, `StopRequested`, `RebuildFailed`) and table tests. Add a
     compare-and-set write in `db` that takes the expected source state. The scanner should
     leave alone the nodes of a lab with an operation in progress. Resume should return an
     enum instead of the "Started" string.
   - **Why first:**
     - every `up` longer than 30 s shows the lab as `Stopped` and offers Start (M1.1);
     - `Failed` cannot be seen (M1.2);
     - `Running` means four different things (M1.3);
     - one mapping table instead of three removes M4.1.
2. **Lab lifecycle across `up`, destroy and clean (M6, M7, M3.1).**
   - **Form:** a stored lab phase, separate from the derived status:
     `Creating → Ready → Destroying → DestroyFailed`. `up` moves it out of `Creating`, and
     destroy and clean move it through `Destroying`.
   - Make the lab row the reservation: insert it as `Creating` before anything else, so a
     second `up` fails before it can roll anything back.
   - Make `clean` succeed when the row is already gone.
   - Make `UpError.is_critical` mean something, or remove it.
   - **Why second:** M3.1 deletes a user's running lab; M6.2, M6.3 and M7.1 strand labs and
     directories with no way back through the API; M6.1 hides partial failures.
3. **P2p link attachment (M25, M8.1, M9.2).**
   - **Form:** one transition function that re-attaches a node's links from the DB rows after
     any event that recreates an end: VM cold boot, unikernel cold boot, container start,
     redeploy. It should collect errors per link instead of stopping at the first. Test it
     with a fake `network` layer.
   - **Why:** each defect breaks the dataplane silently while every node shows `Running`.
4. **Sessions (M14).** *Done in #501: a session table, see M14.*
   - **Form:** tokens that name the user by ID, plus a per-user token version, or a session
     table, checked in `resolve_user`. Logout, password change and user deletion move the
     version on. Re-resolve web jobs when they start.
   - **Why:** these are security defects, and M14.1 and M14.2 are easy to reach.
5. **ZTP settings (M29).**
   - **Form:** replace the `enable` and `method` pair with one enum, for example
     `Disabled | Method(ZtpMethod)`, with no `None` variant. Validate it against the model in
     one place, used by the web form, import and `up` planning.
   - **Why:** M29.1 boots nodes against a DHCP option for a missing file; M29.2 and M29.3 fail
     late or silently.
6. **Node image records (M28).**
   - **Form:** an import that copies to a temporary file, renames it, and only then writes the
     row. Add one rule that keeps or promotes a default, used by import, download, delete and
     the form, with tests.
   - **Why:** M28.1 hands a truncated disk to `up`; M28.2 makes `up` fail with "default not
     found".
7. **Client TLS trust (M21).**
   - **Form:** resolve the settings once into a `TrustMode` enum at config load, and reject
     contradictory combinations with an error. Detect a changed pinned certificate and say so.
   - **Why:** it is easy to misconfigure and hard to diagnose. The ignored CA path is a trap.
8. **Vault init (M11).**
   - **Form:** record an "initialising" marker before `init()` and handle it explicitly, or
     make the credentials write the only way out of that state.
   - **Why:** rare, but the Vault cannot be recovered without a redeploy.
9. **Link impairment (M26).**
   - **Form:** have `remove_netem` tell "none present" apart from other errors, and write the
     database before the kernel or roll back both.
   - **Why:** narrow, but it leaves a link impaired while reporting it clear.
10. **Web jobs and streams (M15, M16).**
    - **Form:** a job record with `Pending → Running → Finished`, an expiry, and a stored
      result that a reconnecting client can read.
    - **Why:** memory held by jobs that are never streamed; results lost on reload or
      disconnect.
11. **Daemon PID file and server certificate (M17, M18).**
    - **Form:** an exclusive lock on the PID file. A certificate check at startup that covers
      expiry and SANs, and warns or regenerates.
    - **Why:** low frequency. The expiry hits every install a year after the certificate was
      generated.
12. **Client lab files and provisioning (M23, M24).**
    - **Form:** record on the client that a lab's local files are pending, and a provisioned
      marker per node.
    - **Why:** each case recovers by hand with `download` or `provision`.

The rest (M10, M12, M13, M19, M20, M27, M30, M31, M32) are low risk. They can be fixed where
they are, without a new type.
