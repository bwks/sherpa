---
name: gh-push
description: Push the current branch to GitHub using gh authentication. Use when the user asks to push commits or publish the current branch.
allowed-tools: bash
---

# Git Push

Push the current branch to GitHub, using `gh` authentication.

## Instructions

### 1. Set up GitHub auth

```bash
gh auth setup-git
```

### 2. Check state

Run `git status` to confirm there are commits to push. Show the current branch name and how many commits ahead of the remote it is.

If the branch has no remote tracking branch yet, push with `-u` to set it up:

```bash
git push -u origin <branch-name>
```

Otherwise push normally:

```bash
git push
```

### 3. Report

Show the push result. If it failed, report the error output.
