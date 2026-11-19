# Lab lifecycle

Bringing a lab up, taking it down and resuming it, destroying it, redeploying one of its nodes, an admin's clean of a lost lab, and operations on labs at the same time. Nodes, links, images, accounts and ZTP have their own specs.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `lab_up_001_a_container_lab_comes_up_with_every_node_running`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above.

The "Today" column names the existing test that becomes the case when it is renamed; "split from" means the case is one check carved out of a bigger test; "none" is a gap with no test yet. The column goes once every case's test carries its ID.

## `lab_up`: up builds a lab from a manifest

**Statement:** `sherpa up` with a valid manifest creates every node and link, starts the nodes, and reports the lab ready, or reports each node that failed and exits non-zero.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level | Today |
|---|---|---|---|
| 001 | A container lab comes up and every node is running | integration | `test_container_lab_up_and_inspect` |
| 002 | A VM lab comes up and its node accepts an SSH login | integration | split from `test_vm_lab_up_and_destroy` |
| 003 | A mixed VM and container lab comes up with its links working | e2e | `tests/labs/p2p-links` |
| 004 | A node that fails to start fails the node, not the lab, and up exits non-zero | integration | `a_container_that_exits_at_once_fails_its_node_not_the_lab` |
| 005 | A container that exits shortly after starting fails with its exit code and log | integration | `a_container_that_exits_a_moment_after_it_starts_fails_with_its_exit_code_and_log` |

## `lab_preflight`: up refuses a lab it can't build, before creating anything

**Statement:** up refuses a request it can't carry out with an error that names the problem, and leaves nothing behind: no lab record, no node, no network.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level | Today |
|---|---|---|---|
| 001 | A manifest naming an image that isn't imported is refused, naming the image | integration | `test_lab_up_missing_image` |
| 002 | An invalid manifest is refused | integration | `test_lab_up_invalid_manifest` |
| 003 | A lab ID already in use is refused | integration | `test_lab_up_rejects_duplicate_lab_id` |
| 004 | An unsupported ZTP setting is refused | integration | `up_refuses_an_unsupported_ztp_setting_before_creating_anything` |
| 005 | CPUs pinned by another lab are refused, naming the lab holding them | integration | `of_two_concurrent_ups_pinning_the_same_cpus_one_builds_and_the_other_creates_nothing` |
| 006 | Not enough free memory is refused | integration | none |
| 007 | A refusal for lack of resources exits with code 75 | e2e | none |

## `lab_concurrency`: operations at once leave each lab consistent

**Statement:** two operations on the same lab, or ups of different labs at once, finish with each lab consistent.
**Relied on by:** the user, scripts. **Priority:** P1.

| Case | What it shows | Level | Today |
|---|---|---|---|
| 001 | A second up of a lab that is coming up is refused and leaves the first alone | integration | `two_ups_of_one_lab_leave_the_first_alone` |
| 002 | Two destroys of one lab both succeed | integration | `two_destroys_of_one_lab_both_succeed` |
| 003 | Labs brought up at once get distinct management subnets | integration | `test_concurrent_lab_up_allocates_distinct_subnets` |

## `lab_down_resume`: down stops a lab and resume brings it back as it was

**Statement:** down stops every node and keeps the lab; resume starts them again with their disks, addresses and links as before, and waits until they are ready, as up does.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level | Today |
|---|---|---|---|
| 001 | A container lab goes down and resumes with its nodes running | integration | `test_container_lab_down_and_resume` |
| 002 | Resume waits for a VM to be ready and streams its progress | e2e | owner-login lab; split from `current_user_vm_login_sudo_resume_and_owner_redeploy` |
| 003 | A resumed VM keeps its guest's keys and the owner can still log in | e2e | owner-login lab; split from `current_user_vm_login_sudo_resume_and_owner_redeploy` |
| 004 | A VM's disk changes survive down and resume | e2e | none |

## `lab_destroy`: destroy removes everything the lab created

**Statement:** destroy removes the lab's nodes, networks, disks, records, folder and CPU pins on the server, and exits non-zero if any part could not be removed.
**Relied on by:** the user, the host's capacity. **Priority:** P0.

| Case | What it shows | Level | Today |
|---|---|---|---|
| 001 | A container lab is destroyed with nothing left in Docker, the database or the lab folder | integration | `test_container_lab_destroy` |
| 002 | A VM lab is destroyed with nothing left in libvirt | integration | split from `test_vm_lab_up_and_destroy` |
| 003 | A destroyed lab's CPU pins can be pinned again | integration | `destroying_a_lab_lets_its_pins_be_pinned_again` |
| 004 | A partial failure is reported and exits non-zero | integration | none |

## `lab_redeploy`: redeploy rebuilds one node in place

**Statement:** redeploy rebuilds a node from its stored definition; a refused redeploy leaves the node as it was, and a failure after teardown fails that node.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level | Today |
|---|---|---|---|
| 001 | A redeployed VM comes back and the owner can log in | e2e | owner-login lab; split from `current_user_vm_login_sudo_resume_and_owner_redeploy` |
| 002 | A refused redeploy leaves the running node untouched | e2e | owner-login lab; split from `current_user_vm_login_sudo_resume_and_owner_redeploy` |
| 003 | A refused redeploy keeps the node's CPU pins | integration | `a_refused_redeploy_keeps_its_pins_and_its_node` |
| 004 | A redeploy to new pins holds its old ones from a racing up until it ends | integration | `a_redeploy_with_new_pins_holds_its_old_ones_from_a_racing_up_until_it_ends` |
| 005 | A redeploy that fails after teardown fails its node | integration | `a_redeploy_that_fails_after_teardown_fails_its_node` |

## `lab_clean`: admin clean removes what a lost lab left behind

**Statement:** an admin's clean removes a lab's resources even when its record or its folder is gone.
**Relied on by:** the admin. **Priority:** P2.

| Case | What it shows | Level | Today |
|---|---|---|---|
| 001 | Clean removes a running lab | integration | `admin_clean_removes_a_running_lab` |
| 002 | Clean removes a lab folder with no record | integration | `admin_clean_removes_a_lab_folder_with_no_record` |
