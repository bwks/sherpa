---
name: gh-branch
description: Create a new git branch and switch to it. Use when the user asks to create or switch to a new GitHub-backed branch.
allowed-tools: bash
---

# Git Branch

Create a new branch and switch to it.

## Arguments

- `<name>` (required): the branch name to create

## Instructions

### 1. Set up GitHub auth

```bash
gh auth setup-git
```

### 2. Validate

If no branch name argument was provided, report the error and stop.

Check that the branch does not already exist locally or on the remote:

```bash
git branch --list <name>
git ls-remote --heads origin <name>
```

If it already exists, report that and stop.

### 3. Create and switch

```bash
git checkout -b <name>
```

### 4. Report

Confirm the new branch was created and is now the active branch.
