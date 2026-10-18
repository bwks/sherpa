# Shared Validators — Test Specifications

> **Module:** `crates/shared/src/validators/`
> **External Dependencies:** None (pure logic)

Validators used by more than one crate. Single-owner validators live in that crate's own
`validate` module, and manifest checks live in `topology::validate`.

---

## Password Strength (`validators/password.rs`)

**What to test:**
- `validate_password_strength()` enforces:
  - Minimum 8 characters `[unit]` **P0**
  - At least one uppercase letter `[unit]` **P0**
  - At least one lowercase letter `[unit]` **P0**
  - At least one special character (!@#$%^&*_+-=) `[unit]` **P0**
- Weak passwords rejected with descriptive error `[unit]` **P0**

---

## SSH Key Validation (`validators/ssh_key.rs`)

**What to test:**
- `validate_ssh_key()` accepts valid ssh-rsa key `[unit]` **P0**
- Accepts valid ssh-ed25519 key `[unit]` **P0**
- Accepts valid ecdsa-sha2-nistp256/384/521 keys `[unit]` **P0**
- Accepts ssh-dss key `[unit]` **P1**
- Rejects key with unknown algorithm `[unit]` **P0**
- Rejects malformed base64 data `[unit]` **P0**
- Rejects key with wrong number of parts `[unit]` **P0**
- Optional comment field accepted `[unit]` **P1**

**Existing coverage:** Inline tests cover core cases

---

## Usernames (`validators/username.rs`)

**What to test:**
- `validate_username()` accepts 3–32 lowercase letters, digits, `_` and `-`, starting with a letter or `_` `[unit]` **P0**
- Rejects short, uppercase, leading digit or hyphen, `.`, `@`, spaces, non-ASCII and 33 characters `[unit]` **P0**
- Rejects every name in `konst::RESERVED_USERNAMES` and the `systemd-` prefix `[unit]` **P0**
- Every reserved name is itself a portable name `[unit]` **P1**
- `util::normalize_username()` trims and lowercases `[unit]` **P0**

Guest usernames (`validate_guest_username()`):
- Portable lowercase names and the 32-character boundary pass `[unit]` **P0**
- Uppercase, leading digit or hyphen, `.`, `@`, non-ASCII, newlines and 33 characters fail `[unit]` **P0**
- `root` and caller-supplied reserved names fail `[unit]` **P0**

---

## Tailscale Auth Key (`validators/tailscale_key.rs`)

**What to test:**
- `tskey-auth-` keys pass `[unit]` **P0**
- Empty, prefix-only, whitespace-containing and OAuth client keys fail `[unit]` **P0**
- Error messages never echo the key `[unit]` **P0**

---

## Data Interface Count (`validators/interface_count.rs`)

**What to test:**
- No manifest override uses the image default `[unit]` **P0**
- A supported override is returned `[unit]` **P0**
- An override beyond the model's interfaces fails with node, model and count `[unit]` **P0**
- Reserved interface overflow fails `[unit]` **P1**
