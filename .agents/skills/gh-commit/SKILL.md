---
name: gh-commit
description: Commit staged and unstaged changes using the repository or global git identity. Use when the user asks to commit current changes.
allowed-tools: bash read
---

# Git Commit

Commit current changes using the existing repository/global Git identity.

Do **not** set or override `user.name`, `user.email`, `GIT_AUTHOR_*`, or `GIT_COMMITTER_*` unless the user explicitly asks. Use the effective Git config as-is.

## Arguments

- No argument: auto-generate a commit message based on the changes
- `<message>`: use the provided text as the commit message

## AI attribution

Keep the human Git author/committer from the effective Git config. Add AI attribution in the commit message trailers instead of changing Git identity.

When the active assistant/model is known, include model details in trailers:

```text
AI-Assisted-By: <provider-or-agent> <model-id-or-model-name>
```

Also include a co-author trailer for the assistant family when it is clear:

- Claude / Anthropic: `Co-authored-by: Claude <noreply@anthropic.com>`
- Codex / OpenAI: `Co-authored-by: Codex <noreply@openai.com>`

If the active model/provider is not known from session context, inspect pi settings if useful:

```bash
if [ -f .pi/settings.json ]; then cat .pi/settings.json; fi
if [ -f ~/.pi/agent/settings.json ]; then cat ~/.pi/agent/settings.json; fi
```

If still unknown, omit AI trailers rather than guessing.

## Instructions

### 1. Set up GitHub auth

```bash
gh auth setup-git
```

### 2. Check identity

Show the effective author identity before committing:

```bash
git config user.name
git config user.email
```

If either value is missing, stop and ask the user to configure Git identity.

### 3. Check for changes

Run `git status` and `git diff` to understand what has changed.

If there are no changes, including untracked files, report that there is nothing to commit and stop.

### 4. Stage changes

Stage modified and untracked files deliberately. Prefer adding specific files by name rather than using `git add -A`.

Never stage files that look like they contain secrets, such as `.env`, private keys, credentials, tokens, or password files. Warn the user if such files are present.

### 5. Create commit message

If a message argument was provided, use it as the commit message.

If no message was provided, analyze the staged changes and draft a concise commit message:

- Use conventional commit format: `fix:`, `feat:`, `chore:`, `refactor:`, `test:`, or `docs:`
- First line under 72 characters summarizing the why
- Optional body with more detail if the change is non-trivial
- Check `git log --oneline -5` to match the repository's style

### 6. Commit

Create the commit using the effective Git identity. Add the AI attribution trailers described above when the active model/provider is known. Do not set the commit author/committer to the AI identity.

Use a HEREDOC to pass multi-line messages:

```bash
git commit -m "$(cat <<'EOF'
<commit message here>

AI-Assisted-By: <provider-or-agent> <model-id-or-model-name>
Co-authored-by: <assistant-family> <assistant-email>
EOF
)"
```

Omit the `Co-authored-by` line if the assistant family is not Claude/Anthropic or Codex/OpenAI. Omit both AI trailer lines if the model/provider is unknown.

### 7. Report

Show the commit hash, author, committer, and summary. If the commit failed, report the error output.
