#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""Host memory evidence for the VM code suite (Python 3.15+, standard library only).

Two things the release runner reads from the Sherpa server:

- a `sherpa up` the server refused for memory (the CLI exits 75), which the suite runs again on
  its own once the other releases finish;
- a test guest the host's OOM killer killed, which the runner reports instead of the dropped
  SSH connection's `exited 255`.

The functions here take the commands' output as text, so the classification is tested without a
host. `vm_release_test.py` runs the commands; `test_env.py` schedules the refused releases.
"""

from dataclasses import dataclass
import re
import sys
import unittest


# `sherpa`'s exit code for a command the server refused for memory: the server's InsufficientMemory
# JSON-RPC error (-32005), mapped in `crates/client/src/cmd/exit_status.rs`.
SHERPA_MEMORY_REFUSED = 75
# The runner's exit code for a release the server refused for memory (EX_TEMPFAIL).
MEMORY_REFUSED_EXIT = 75
# The kernel's one-line summary of an OOM kill, naming the victim's cgroup.
OOM_KILL = "oom-kill:"
# libvirt puts each QEMU domain in a systemd scope under this slice. Its memory.events counts the
# OOM kills of every process below it, and stays readable after the domain's scope is gone.
MACHINE_SLICE_EVENTS = "/sys/fs/cgroup/machine.slice/memory.events"
# What journalctl prints to a user outside the groups that may read the system journal.
JOURNAL_DENIED = "not seeing messages from other users"
# What dmesg prints when kernel.dmesg_restrict hides the ring buffer.
DMESG_DENIED = "Operation not permitted"
# libvirt's machine names are at most this long (virDomainMachineNameAppendValid).
MACHINE_NAME_LIMIT = 64
HOSTNAME_CHARS = frozenset("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789")
ESCAPE = re.compile(r"\\x([0-9a-fA-F]{2})")
SCOPE = re.compile(r"^machine-qemu-(\d+)-.+\.scope$")


@dataclass(frozen=True)
class GuestLoss:
    """Why a test guest stopped: kind is "oom-killed" or "domain-stopped"."""
    kind: str
    summary: str
    evidence: tuple[str, ...]


def memory_refusal(code, output):
    """The server's memory refusal of a failed `sherpa up`, or an empty string.

    The exit code decides; the output only describes it. The CLI prints the error last.
    """
    if code != SHERPA_MEMORY_REFUSED:
        return ""
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    return lines[-1] if lines else f"sherpa up exited {code}"


def machine_name(domain_id, domain):
    """libvirt's systemd machine name for a QEMU domain (virDomainDriverGenerateMachineName)."""
    name = f"qemu-{domain_id}-"
    skip = True
    for character in domain:
        if len(name) >= MACHINE_NAME_LIMIT:
            break
        if character in ".-":
            if not skip:
                name += character
            skip = True
            continue
        skip = False
        if character in HOSTNAME_CHARS:
            name += character
    return name.rstrip("-.")


def unescape_unit(name):
    """A systemd unit name with its \\xNN escapes decoded."""
    return ESCAPE.sub(lambda match: chr(int(match.group(1), 16)), name)


def oom_kill_fields(line):
    """The key=value fields of a kernel oom-kill line."""
    fields = {}
    for part in line[line.index(OOM_KILL) + len(OOM_KILL):].split(","):
        key, separator, value = part.partition("=")
        if separator:
            fields[key.strip()] = value.strip()
    return fields


def killed_scope(line):
    """The domain id and machine name of the libvirt scope an oom-kill line's victim was in."""
    for component in oom_kill_fields(line).get("task_memcg", "").split("/"):
        scope = unescape_unit(component)
        match = SCOPE.match(scope)
        if match:
            return int(match.group(1)), scope.removeprefix("machine-").removesuffix(".scope")
    return None


def oom_kill_line(kernel_log, domain):
    """The latest kernel log oom-kill line whose victim was in the domain's QEMU scope, or ''."""
    for line in reversed(kernel_log.splitlines()):
        if OOM_KILL not in line:
            continue
        scope = killed_scope(line)
        if scope and scope[1] == machine_name(scope[0], domain):
            return line.strip()
    return ""


def kernel_log_denied(journal, dmesg):
    """Whether neither journalctl nor dmesg could read the kernel log."""
    return JOURNAL_DENIED in journal and DMESG_DENIED in dmesg


def oom_kills(memory_events):
    """The oom_kill count in a cgroup's memory.events, or None when it has none."""
    for line in memory_events.splitlines():
        key, _, value = line.partition(" ")
        if key == "oom_kill" and value.strip().isdigit():
            return int(value)
    return None


def domain_stopped(state):
    """Whether `virsh domstate --reason` says the domain is no longer running."""
    return state.strip().startswith(("shut off", "crashed"))


def classify_guest_loss(domain, state, journal, dmesg, kills_before, kills_after):
    """Why the domain stopped, from the host's evidence, or None while it still runs.

    The kernel log's oom-kill line naming the domain's scope is proof. Without a readable kernel
    log, a domain libvirt saw crash while the OOM kills under machine.slice rose is reported as
    killed too, with that evidence.
    """
    state = state.strip()
    if not domain_stopped(state):
        return None
    domstate = f"virsh domstate --reason {domain}: {state}"
    line = oom_kill_line(journal, domain) or oom_kill_line(dmesg, domain)
    if line:
        return GuestLoss("oom-killed", f"{domain} was killed by the host OOM killer: {line}", (domstate, line))
    if (kernel_log_denied(journal, dmesg) and "crashed" in state
            and kills_before is not None and kills_after is not None and kills_after > kills_before):
        counter = f"{MACHINE_SLICE_EVENTS} oom_kill {kills_before} -> {kills_after}"
        denied = ("the kernel log is not readable by this user (journalctl needs the adm or "
                  "systemd-journal group, dmesg needs kernel.dmesg_restrict=0)")
        return GuestLoss("oom-killed",
                         f"{domain} was killed by the host OOM killer: libvirt saw it crash while {counter}; "
                         f"{denied}", (domstate, counter, denied))
    return GuestLoss("domain-stopped", f"{domain} stopped on the host ({state}); no oom-kill line names it",
                     (domstate,))


# A kernel memcg OOM kill of a QEMU domain, captured on a Sherpa server.
MEMCG_KILL = ("2026-11-12T19:40:40+00:00 agent-2 kernel: oom-kill:constraint=CONSTRAINT_MEMCG,"
              "nodemask=(null),cpuset=libvirtd.service,mems_allowed=0,"
              "oom_memcg=/machine.slice/machine-qemu\\x2d9\\x2dsmall\\x2dcb2dd9af.scope/libvirt,"
              "task_memcg=/machine.slice/machine-qemu\\x2d9\\x2dsmall\\x2dcb2dd9af.scope/libvirt,"
              "task=qemu-system-x86,pid=714630,uid=64055")
# A host-wide OOM kill, in the form #514 recorded.
GLOBAL_KILL = ("2026-11-09T09:21:51+00:00 host kernel: oom-kill:constraint=CONSTRAINT_CPUSET,"
               "nodemask=(null),cpuset=iothread1,mems_allowed=0,global_oom,"
               "task_memcg=/machine.slice/machine-qemu\\x2d41\\x2drt1a2b3c4d\\x2d5e6f7a8b.scope/libvirt/emulator,"
               "task=qemu-system-x86,pid=4242,uid=64055")
JOURNAL_HINT = ("Hint: You are currently not seeing messages from other users and the system.\n"
                "      Users in groups 'adm', 'systemd-journal' can see all messages.\n"
                "      Pass -q to turn off this notice.\n-- No entries --\n")
DMESG_HINT = "dmesg: read kernel buffer failed: Operation not permitted\n"
UP_REFUSED = """Manifest Validation
───────────────────
🔄 Validating manifest structure
Lab creation failed: Not enough memory on the server: node 'bigb' needs 16384 MiB, but only 11614 MiB of guest memory is left, 4770 MiB short. (code: -32005)
"""


class HostMemoryTests(unittest.TestCase):
    def test_oom_kill_line_names_the_domains_scope(self):
        log = "\n".join(["kernel: rpc-libvirtd invoked oom-killer: gfp_mask=0xcc0", MEMCG_KILL, GLOBAL_KILL])
        self.assertEqual(oom_kill_line(log, "small-cb2dd9af"), MEMCG_KILL)
        self.assertEqual(oom_kill_line(log, "rt1a2b3c4d-5e6f7a8b"), GLOBAL_KILL)
        for other in ("small-cb2dd9a", "mall-cb2dd9af", "rt1a2b3c4d-5e6f7a8"):
            with self.subTest(domain=other):
                self.assertEqual(oom_kill_line(log, other), "")

    def test_a_stopped_domain_without_oom_evidence_is_not_an_oom_kill(self):
        cases = (("shut off (crashed)", JOURNAL_HINT, DMESG_HINT, 1, 1),     # no new kill
                 ("shut off (crashed)", JOURNAL_HINT, DMESG_HINT, None, 2),  # no baseline
                 ("shut off (destroyed)", JOURNAL_HINT, DMESG_HINT, 0, 1),   # not a crash
                 ("shut off (crashed)", "-- No entries --", "", 0, 1),       # readable, no line
                 ("shut off (crashed)", GLOBAL_KILL, "", 0, 1))              # another domain
        for state, journal, dmesg, before, after in cases:
            with self.subTest(state=state, journal=journal, before=before):
                loss = classify_guest_loss("small-cb2dd9af", state, journal, dmesg, before, after)
                self.assertEqual(loss.kind, "domain-stopped")
                self.assertIn(state, loss.summary)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        sys.exit("Usage: host_memory.py --self-test")
