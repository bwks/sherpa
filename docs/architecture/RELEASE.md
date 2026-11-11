# Versions and releases

Sherpa has one version, and it changes only when a release is cut. Pull requests do not change
it.

## The version

The version is `[workspace.package] version` in the root `Cargo.toml`. Every workspace crate
inherits it with `version.workspace = true`, so `env!("CARGO_PKG_VERSION")` gives the same value
everywhere it is read: the CLI's and `sherpad`'s `--version` (`crates/client/src/cmd/cli.rs`,
`crates/server/src/cli.rs`), the server's system handler (`crates/server/src/api/handlers/system.rs`)
and the client/server version check (`crates/client/src/ws_client/client.rs`,
`crates/server/src/api/websocket/handler.rs`). A build of main between releases reports the
last released version.

`crates/ebpf-redirect` is outside the workspace, with its own lockfile, and is not versioned with
Sherpa. Its program is built by hand and its ELF is committed; nothing reads its version, which
changes only when the program does.

## Cutting a release

A release is cut by running the **Prepare release** workflow (`.github/workflows/prepare-release.yml`)
from the Actions tab, or with `gh workflow run prepare-release.yml`. Its inputs:

- `version`: the version to release, `X.Y.Z`. When empty, the workflow works it out with
  `git cliff --bumped-version` from the conventional commits since the last `v*` tag. The
  `[bump]` settings in `cliff.toml` keep 0.x numbering: a `feat` or `fix` commit, or any other,
  bumps the patch number; a breaking change (`feat!:` or a `BREAKING CHANGE` footer) bumps the
  minor number.
- `dry_run`: work out the version, the lockfile and the changelog, and show the files changed
  and the release notes in the run summary, without opening the pull request.

The workflow refuses a version that is not `X.Y.Z`, not higher than the last tag, or already
tagged, and stops with "nothing to release" when there are no commits since the last tag. It
sets the workspace version, runs `cargo update --workspace` (which updates only the workspace
crates' lockfile entries), regenerates `CHANGELOG.md` with git-cliff, and checks that only
`Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` changed.

It then opens the **release pull request** with a token from the release GitHub App
(`SHERPA_RELEASE_APP_CLIENT_ID`, `SHERPA_RELEASE_APP_PRIVATE_KEY`): branch `release/vX.Y.Z`, one
commit `chore(release): vX.Y.Z`, and the release notes in its body. A pull request opened by the
app, unlike one opened with `GITHUB_TOKEN`, runs the PR checks. A second run for the same version
replaces the branch and the pull request's body; a run for another version closes the older
release pull request. Runs queue in the `prepare-release` concurrency group, a new one cancelling
one in progress.

The user merges the release pull request. If main moves on before it merges, run the workflow
again; otherwise the commits merged in between are released and in the release notes, but are
only added to `CHANGELOG.md` by the next release, whose full regeneration places them.

## The release

Every push to main runs `.github/workflows/ci.yml`. Its `check-version` job reads the workspace
version with `cargo metadata` and, when `vX.Y.Z` is not tagged, calls
`.github/workflows/release.yml`. Only the release pull request changes the version, so every
other push finds it tagged and releases nothing.

`release.yml`:

1. `tag-check` looks for the tag on the remote again. Release runs queue in the `release`
   concurrency group, so a second push at the same version, which passed `check-version` before
   the first run tagged it, finds the tag and stops.
2. `build-app` builds `sherpa` for Linux, macOS (x86_64 and aarch64) and Windows, and `sherpad`
   for Linux, at the pushed commit.
3. `release` writes the release notes with git-cliff, tags the pushed commit (`$GITHUB_SHA`),
   not main's tip, which may have moved on, and creates the GitHub release with the archives.

The release commit already carries `CHANGELOG.md`, so the release makes no commit of its own.

## The Version check

`pr.yml`'s `Version` job runs `.github/scripts/check_version.py` on every pull request, against
the base the pull request merges into. The script uses only the standard library, and its inline
metadata (PEP 723) requires Python 3.15, so the job installs uv with `astral-sh/setup-uv` and runs
it with `uv run`: on a uv-managed Python rather than the runner's `python3`, in its own environment,
without syncing the repository's uv project. It fails when:

- a workspace crate sets its own version instead of inheriting it;
- `Cargo.lock`'s entries for the workspace crates are not at the workspace version;
- the pull request changes the version and is not a release pull request.

A release pull request is one authored by `bwks-sherpa-release[bot]`, from a `release/v*` branch
of this repository. Only the release app's token can open a pull request as that author; a
label, a title or a branch name alone could be set by others. A release pull request must also
set the version its branch names, higher than the last tag, and change only `Cargo.toml`,
`Cargo.lock` and `CHANGELOG.md`, which also catches other commits pushed to its branch.

`Version` is a required check in the `main-protect` ruleset, beside `Lint`, `Coverage` and
`Windows Client`. Repository admins can bypass the ruleset, and with it the check.
