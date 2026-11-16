# Console lab test

Tests `sherpa console` end to end through the `sherpa` CLI, in a pseudo-terminal as a user's
terminal runs it. The console is reached through the Sherpa server with the client's own telnet,
and every command runs with the directories holding a system `telnet` left off `PATH`.

| Node | Model | Role |
|---|---|---|
| `alp01` | `alpine_linux` (the default version) | VM whose serial console is opened |
| `ctr01` | `generic_container` (`alpine` 3.22) | Container, which has no serial console |

The runner writes the manifest for each run itself; there is no manifest file here.

## What it checks

1. `sherpa up`.
2. `sherpa console ctr01` fails with the message that containers have no serial console.
3. `sherpa console alp01` prints its connect line; Enter is pressed until `login: ` shows, and
   typed text is echoed by the guest (the line is cleared, never submitted, so no login name or
   password is ever sent).
4. With that console open, a second `sherpa console alp01` fails, saying another client holds
   the console.
5. `Ctrl+]` ends the first console with exit code 0 and one close line, starting on a new line.
6. A new console, at the login prompt again, ends with Enter and `~.` the same way.
7. `sherpa destroy`, and the lab is gone.

## Running it

It runs wherever the CLI is logged in, with a device password set
(`sherpa server user device-password`; the run stops early without one), to a server with the `alpine_linux` image and the
`generic_container` `alpine` 3.22 image; it needs no Docker access. `sherpa` must not be in the
same directory as `telnet`.

```
uv run tests/labs/console/console_lab_test.py
uv run tests/labs/console/console_lab_test.py --keep-failed   # leave a failed lab for debugging
uv run tests/labs/console/console_lab_test.py --self-test     # the runner's own unit tests
```

Logs, including each console's terminal transcript, are under
`.tmp/lab-tests/console-<run>/`. A run takes about 3–5 minutes.
