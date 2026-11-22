# Versions and releases

Sherpa has one version. It changes only in a bump pull request, and the Release workflow ships
main at whatever version main has. A merge never releases.

## The version

The version is `[workspace.package] version` in the root `Cargo.toml`. Every workspace crate
inherits it, so `env!("CARGO_PKG_VERSION")` is the same everywhere: both binaries' `--version`,
the server's system handler and the client/server version check. A build of main after a bump
reports the bumped version, released or not.

`crates/ebpf-redirect` is outside the workspace and not versioned with Sherpa. Its program is
built by hand and its ELF is committed; nothing reads its version, which changes only when the
program does.

## Cutting a release

1. On a branch from an up-to-date main, run `mise run release:bump [X.Y.Z]`.
2. Commit the three files it changes as `chore(release): vX.Y.Z` (git-cliff leaves that message
   out of the changelog), open a pull request and merge it.
3. Run the **Release** workflow on main: the Run workflow button, or `gh workflow run release.yml`.

### `release:bump`

`dev/release-bump` takes the given version or works it out with `git cliff --bumped-version`.
The `[bump]` settings in `cliff.toml` keep 0.x numbering: a breaking change bumps the minor
number, every other commit the patch. It refuses a dirty tree and a version that is not higher
than the last `v*` tag or is already tagged, then sets the version, updates only the workspace
crates in `Cargo.lock`, and prepends the new section to `CHANGELOG.md`. It does not commit.

It writes only the new section and checks that everything after it is the committed file byte
for byte, so release notes written by hand into an earlier section, such as "Upgrade notes",
are never regenerated away. To add notes to the new release, edit its section in the bump pull
request; the Release workflow publishes it as it is on main.

The changelog links pull requests and authors through the GitHub API, so the script needs a
read-only token in `GH_TOKEN_RO` (or `GITHUB_TOKEN`); never the `gh` login token, which can write.

### Caveat: the changelog is written at bump time

Pull requests merged between the bump and the Release run ship with no changelog lines, ever,
as the next bump starts from that release's tag. Keep the gap short: bump, merge, release.

## The release

`.github/workflows/release.yml` runs only on `workflow_dispatch`, in one concurrency group, so
runs queue. Every job checks out `GITHUB_SHA`, main's tip at the click. `check` fails off main,
when `vX.Y.Z` is already tagged, or when `CHANGELOG.md` has no `## [X.Y.Z]` section, which is
the release notes; so a run without a bump, or a second run at the same version, fails there.
`build-app` builds the CLI for Linux, macOS and Windows and `sherpad` for Linux. `release` tags
`GITHUB_SHA`, not main's tip, which may have moved on, and publishes the release. Nothing in CI
commits or generates the changelog.

## The Version check

`pr.yml`'s `Version` job runs `.github/scripts/check_version.py` on every pull request, against
its base, after its tests. It fails when a workspace crate sets its own version, when
`Cargo.lock` disagrees with the workspace version, or when the pull request changes the version
and is not a valid bump: one that changes only `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md`, to
a version higher than the last `v*` tag, with a `## [X.Y.Z]` section for it. Who opens it does
not matter. `Version` is a required check in the `main-protect` ruleset, beside `Lint`,
`Coverage` and `Windows Client`; repository admins can bypass the ruleset.

## Docs-only pull requests

`pr.yml`'s first job, `Changes`, outputs `code=false` when every changed path is docs: a `*.md`
file anywhere, or anything under `docs/` or `tests/specs/`; a rename counts both paths. `Lint`,
`Coverage`, `Windows Client` and `macOS Client` skip on `code=false`, and a job skipped by its
`if:` counts as passed for a required check. If `Changes` fails, its output is empty and the
build jobs run, so a broken detection never skips them. A push to main always outputs
`code=true`, so main keeps saving the build caches pull requests restore. `Version` always runs.

Commits never carry `[skip ci]`: it stops `pr.yml` from starting at all, so the required checks
never report and the pull request stays blocked until an admin bypasses the ruleset.
