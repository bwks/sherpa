# SSH Config Management — Test Specifications

> **Crate:** `crates/shared/`
> **Module:** `util/ssh.rs`
> **External Dependencies:** Filesystem (temp directories for test isolation)
> **Existing Tests:** None

---

## `add_lab_ssh_include`

**What to test:**
- Creates `~/.ssh/sherpa_lab_hosts` if it does not exist `[unit]` **P0**
- Creates `~/.ssh/` directory if it does not exist `[unit]` **P1**
- Adds `Include` line for the lab SSH config path to `sherpa_lab_hosts` `[unit]` **P0**
- Prepends `Include ~/.ssh/sherpa_lab_hosts` to `~/.ssh/config` if not already present `[unit]` **P0**
- Does not duplicate Include in `~/.ssh/config` on repeated calls `[unit]` **P0**
- Does not duplicate lab entry in `sherpa_lab_hosts` on repeated calls `[unit]` **P0**
- Preserves existing content in `~/.ssh/config` `[unit]` **P0**
- Multiple labs produce multiple Include lines in `sherpa_lab_hosts` `[unit]` **P1**

---

## `remove_lab_ssh_include`

**What to test:**
- Removes the correct Include line from `sherpa_lab_hosts` `[unit]` **P0**
- Leaves other lab Include lines intact `[unit]` **P0**
- No error when `sherpa_lab_hosts` does not exist `[unit]` **P1**
- No error when the lab Include line is not present `[unit]` **P1**
- Does not modify `~/.ssh/config` (Include to `sherpa_lab_hosts` stays permanent) `[unit]` **P1**

---

## SSH Config Template (scoped host names)

**What to test:**
- Host line uses `<node>.<lab-id>` format `[unit]` **P0**
- Host FQDN uses `<node>.<lab-id>.<domain>` format `[unit]` **P0**
- Lab ID passed through to template correctly `[unit]` **P0**

**Existing coverage:** 1 test in `crates/template/tests/ssh/mod.rs` (updated for lab_id)

---

## Client-side absolute IdentityFile path

**What to test:**
- Client rewrites relative `IdentityFile sherpa_ssh_key` to absolute path before writing `[unit]` **P0**
- Server-side SSH config retains relative IdentityFile (not modified) `[unit]` **P1**
