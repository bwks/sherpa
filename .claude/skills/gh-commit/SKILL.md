---
name: gh-commit
description: Commit staged and unstaged changes as the checkout's own git identity, with a co-author trailer for the model, using GH_TOKEN for auth
allowed-tools: Bash, Read, Grep, Glob
model: haiku
---

# Git Commit

Commit all current changes as the checkout's own git identity: the person the checkout belongs to is the author, and the model is credited in a `Co-Authored-By` trailer.

## Arguments

- No argument: auto-generate a commit message based on the changes
- `<message>`: use the provided text as the commit message

## Instructions

### 1. Check git identity

Never set or change `user.name` or `user.email`: not with `git config`, not with `-c` flags, not with `--author`. The commit uses the identity the checkout already has.

```bash
git config user.name
git config user.email
```

If either is empty, stop and report that the checkout has no git identity; don't commit.

### 2. Set up GitHub auth

```bash
gh auth setup-git
```

### 3. Check for changes

Run `git status` and `git diff` to understand what has changed.

If there are no changes (no untracked files and no modifications), report that there is
nothing to commit and stop.

### 4. Stage changes

Stage all modified and untracked files. Prefer adding specific files by name rather than
using `git add -A`. Never stage files that look like they contain secrets (.env,
credentials.json, etc.) - warn the user if such files are present.

### 5. Create commit message

If a message argument was provided, use it as the commit message.

If no message was provided, analyse the staged changes and draft a concise commit message:
- Use conventional commit format (fix:, feat:, chore:, refactor:, test:, docs:)
- First line under 72 characters summarising the "why"
- Optional body with more detail if the change is non-trivial
- Check `git log --oneline -5` to match the repository's style

### 6. Commit

Create the commit. Always append a co-author trailer naming the model that is writing the
commit, in the form `Co-Authored-By: <model name> <noreply@anthropic.com>`. Use the exact
model name given in the session's attribution instruction.

Use a HEREDOC to pass the message:

```bash
git commit -m "$(cat <<'EOF'
<commit message here>

Co-Authored-By: <model name> <noreply@anthropic.com>
EOF
)"
```

### 7. Report

Show the commit hash and summary. If the commit failed (e.g. pre-commit hook), report
the error output.
