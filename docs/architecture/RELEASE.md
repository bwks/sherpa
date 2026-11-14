# Versions and releases

Sherpa has one version. It changes only in a bump pull request, and the Release workflow ships
main at whatever version main has. A merge never releases.

## The version

The version is `[workspace.package] version` in the root `Cargo.toml`. Every workspace crate
inherits it with `version.workspace = true`, so `env!("CARGO_PKG_VERSION")` gives the same value
everywhere it is read: the CLI's and `sherpad`'s `--version` (`crates/client/src/cmd/cli.rs`,
`crates/server/src/cli.rs`), the server's system handler (`crates/server/src/api/handlers/system.rs`)
and the client/server version check (`crates/client/src/ws_client/client.rs`,
`crates/server/src/api/websocket/handler.rs`). A build of main after a bump reports the bumped
version, released or not.

`crates/ebpf-redirect` is outside the workspace, with its own lockfile, and is not versioned with
Sherpa. Its program is built by hand and its ELF is committed; nothing reads its version, which
changes only when the program does.

## Cutting a release

1. On a branch from an up-to-date main, run `mise run release:bump [X.Y.Z]`.
2. Commit the three files it changes as `chore(release): vX.Y.Z` (git-cliff skips that message in
   the changelog), open a pull request and merge it.
3. Run the **Release** workflow on main: the Run workflow button in the Actions tab, or
   `gh workflow run release.yml`.

### `release:bump`

The task is defined in the root `mise.toml` and runs `dev/release-bump`. It:

- refuses to run on a working tree with uncommitted changes, and fetches the tags from `origin`;
- takes the version given, or works it out with `git cliff --bumped-version` from the
  conventional commits since the last `v*` tag. The `[bump]` settings in `cliff.toml` keep 0.x
  numbering: a `feat` or `fix` commit, or any other, bumps the patch number; a breaking change
  (`feat!:` or a `BREAKING CHANGE` footer) bumps the minor number. With no commits since the
  last tag it stops;
- refuses a version that is not `X.Y.Z`, not higher than the last `v*` tag, or already tagged,
  locally or on `origin`;
- sets the workspace version and runs `cargo update --workspace`, which updates only the
  workspace crates' lockfile entries;
- adds the new version's section to the top of `CHANGELOG.md` with
  `git cliff --unreleased --tag vX.Y.Z --prepend CHANGELOG.md`: the commits since the last tag,
  under the header, which git-cliff keeps once. The earlier sections are not regenerated;
- checks the changelog against the committed one (`git show HEAD:CHANGELOG.md`): the header is
  the same, the first section is the new version's and ends with one blank line, and the text
  after it is the committed file's text after its header, byte for byte. Otherwise it stops,
  shows the diff and leaves `CHANGELOG.md` as written;
- checks that only `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` changed, and prints the diff
  stat and the new changelog section.

It does not commit. git-cliff is installed by mise, pinned in the root `mise.toml` and in the
dev server's `tests/environments/dev/provision/mise.toml`.

The changelog links each entry's pull request and author through the GitHub API, which needs a
token. The repository is set in `cliff.toml` (`[remote.github]`); the script passes git-cliff a
read-only token from `GH_TOKEN_RO`, or uses `GITHUB_TOKEN` if that is already set, and stops when
neither is. Guest agents have `GH_TOKEN_RO`. On the host, set `GH_TOKEN_RO` to a fine-grained
personal access token with read-only access to the repository (no write permissions); do not
use the `gh` login token, which can write.

### Hand-written release notes

Notes written by hand into a section, such as a release's "Upgrade notes" or "Breaking changes"
block, are kept: a bump writes only the new section and leaves every earlier one as it is. To
add notes to a release, edit its section in the bump pull request, after running the bump; the
Release workflow publishes the section as it is on main. The check runs only in the script and
compares with the committed file, so an earlier section edited and committed before the bump
stays as committed; nothing in CI checks the changelog beyond the new version's heading.

`trim = true` in `cliff.toml` strips the blank line at the end of each section, so the body
template ends with a marker that a postprocessor removes after the trim. That keeps one blank
line between the new section and the previous version's heading.

### Caveat: the changelog is written at bump time

`CHANGELOG.md` is written by the bump, not by the release. Pull requests merged after the bump
pull request and before the Release run ship in that release, but have no lines in its section
or its release notes. The next bump lists only the commits after that release's tag, so those
pull requests never get changelog lines. Keep the gap short: bump, merge, release.

## The release

`.github/workflows/release.yml` runs only on `workflow_dispatch`, in the `release` concurrency
group, so runs queue rather than overlap. Every job checks out the commit the run started at
(`GITHUB_SHA`), main's tip at the click.

1. `check` fails unless the run is on main, reads the version with `cargo metadata`, fails if
   `vX.Y.Z` is already tagged on the remote, and fails if `CHANGELOG.md` has no `## [X.Y.Z]`
   section. That section, without its heading, is the release notes. A second run at the same
   version, queued behind the first, finds the tag and fails here.
2. `build-app` builds `sherpa` for Linux, macOS (x86_64 and aarch64) and Windows, and `sherpad`
   for Linux.
3. `release` tags `GITHUB_SHA`, not main's tip, which may have moved on, pushes the tag with the
   workflow's token, and creates the GitHub release with the archives and the notes.

The release makes no commit and opens no pull request; nothing in CI generates the changelog.
Clicking Release without a bump fails at the tag check.

## The Version check

`pr.yml`'s `Version` job runs `.github/scripts/check_version.py` on every pull request, against
the base the pull request merges into, after `.github/scripts/test_check_version.py`, its tests.
Both use only the standard library, and their inline metadata (PEP 723) requires Python 3.15, so
the job installs uv with `astral-sh/setup-uv` and runs them with `uv run`: on a uv-managed Python
rather than the runner's `python3`, in their own environment, without syncing the repository's
uv project. The check fails when:

- a workspace crate sets its own version instead of inheriting it;
- `Cargo.lock`'s entries for the workspace crates are not at the workspace version;
- the pull request changes the version and is not a valid bump.

A bump pull request changes nothing but `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md`, sets a
version higher than the last `v*` tag, and has a `## [X.Y.Z]` section for it in `CHANGELOG.md`.
Any pull request that meets those rules may change the version, whoever opens it; every other
pull request must leave it alone.

`Version` is a required check in the `main-protect` ruleset, beside `Lint`, `Coverage` and
`Windows Client`. Repository admins can bypass the ruleset, and with it the check.
