# Token Management — Test Specifications

> **Crate:** `crates/client/` (`token.rs`)
> **External Dependencies:** Filesystem (~/.sherpa/)
> **Existing Tests:** 3 ignored tests (filesystem-dependent)

---

## Token Persistence

**What to test:**
- `save_token()` creates ~/.sherpa directory if missing `[unit]` **P0**
- `save_token()` writes token to ~/.sherpa/token `[unit]` **P0**
- Token file created with 0600 permissions (owner read/write only) `[unit]` **P0**
- `load_token()` reads and trims whitespace from token `[unit]` **P0**
- Save/load round-trip preserves token content `[unit]` **P0**

---

## Token Absence

**What to test:**
- `load_token()` when no file exists returns "No token found" error `[unit]` **P0**
- `token_exists()` returns false when no file `[unit]` **P1**
- `token_exists()` returns true after save `[unit]` **P1**

---

## Token Deletion

**What to test:**
- `delete_token()` removes the token file `[unit]` **P0**
- `delete_token()` is idempotent (no error if file already missing) `[unit]` **P0**

**Existing coverage:** 3 tests (ignored) cover save/load round-trip, missing file, and path derivation
