# Manifest commands test

Tests what the manifest commands of the `sherpa` CLI print and exit with: `sherpa validate`,
`sherpa new`, and a `sherpa up` the server refuses before it creates anything. It brings no lab
up, so it needs no node image and runs in seconds.

## Requirements

- The `sherpa` CLI on `PATH`. Every case but `validate_command_005` works offline;
  `validate_command_001` runs `sherpa validate` with an empty home and none of the CLI's
  `SHERPA_` variables, as on a machine with no login and no server.
- For `validate_command_005`, the CLI logged in, as the other lab runners are, with an SSH key
  and a device password (`sherpa server user device-password`): the server checks both before it
  reads the manifest's nodes. The `logged_in` fixture checks the login only when that case runs,
  so `-k 'not validate_command_005'` runs the rest with no server. No account is created.

## Running

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header
and runs pytest on it, passing on any other arguments:

```sh
uv run tests/labs/manifest-commands/manifest_commands_lab_test.py
uv run tests/labs/manifest-commands/manifest_commands_lab_test.py -k manifest_file_011
uv run tests/labs/manifest-commands/manifest_commands_lab_test.py --self-test  # runner unit tests, seconds
```

pytest runs it directly too, for example `uvx --python 3.15 pytest==9.2.0
tests/labs/manifest-commands/manifest_commands_lab_test.py`.

The module-scoped `prepared` fixture makes the run's folder,
`.tmp/lab-tests/manifest-commands-<id>/`; each case writes its manifest in a folder of its own
under it, and the run's lab names start with the folder's name. At the end the fixture removes
the case folders and keeps `commands.log`, which records every command and its output. The cases
are the functions named `test_<case_id>_*`, in the order below; every case runs even when one
fails. The runner's own unit tests, under `--self-test`, are not collected by pytest.

## What it checks

It holds the command cases of [`tests/specs/manifests.md`](../../specs/manifests.md):

| Case | Check |
|---|---|
| `manifest_file_001` | `sherpa validate` in a folder with no `manifest.toml` exits 1 and says the file was not found |
| `manifest_file_009` | `sherpa validate` of a manifest with an unknown key on a node exits 1 and prints the file, the line and column, the key and the keys a node takes |
| `manifest_file_010` | `sherpa new` writes two Linux nodes and a link between them under a generated name, and `sherpa validate` passes it |
| `manifest_file_011` | `sherpa new` leaves an existing `manifest.toml` as it is, says so and exits non-zero, and `sherpa new --force` writes over it |
| `validate_command_001` | `sherpa validate` of a valid manifest, with an empty home and no `SHERPA_` variables, ticks every check, prints `Manifest validation passed!` and exits 0 |
| `validate_command_002` | `sherpa validate` of a 1001-character description exits 1 with the description check's message, and prints no check after it |
| `validate_command_005` | `sherpa up` of a node whose `environment_variables` key starts with a digit, which only the server checks, exits 1 with the server's message naming the key and the node, and writes no `lab-info.toml`; should up ever pass it, the case destroys what it built |

That a refused up creates nothing on the server is `lab_preflight_002`, in
[`tests/specs/lab-lifecycle.md`](../../specs/lab-lifecycle.md).
