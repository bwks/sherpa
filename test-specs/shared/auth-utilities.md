# Shared Auth Utilities — Test Specifications

> **Crate:** `crates/shared/` (`auth/`)
> **External Dependencies:** None (pure crypto)
> **Existing Tests:** Inline tests in jwt.rs, password.rs, ssh.rs

---

## Password Hashing (`auth/password.rs`)

**What to test:**
- `hash_password()` produces Argon2id PHC format hash `[unit]` **P0**
- `verify_password()` returns true for correct password `[unit]` **P0**
- `verify_password()` returns false for incorrect password `[unit]` **P0**
- Different calls with same password produce different hashes (random salt) `[unit]` **P0**
- `validate_password_strength()` enforces:
  - Minimum 8 characters `[unit]` **P0**
  - At least one uppercase letter `[unit]` **P0**
  - At least one lowercase letter `[unit]` **P0**
  - At least one special character (!@#$%^&*_+-=) `[unit]` **P0**
- Weak passwords rejected with descriptive error `[unit]` **P0**
- Constant-time comparison used in verification `[unit]` **P2**

---

## JWT Operations (`auth/jwt.rs`)

**What to test:**
- `Claims::new()` sets correct iat (now) and exp (now + expiry_seconds) `[unit]` **P0**
- `Claims::is_expired()` returns false for fresh token `[unit]` **P0**
- `Claims::is_expired()` returns true for past expiry `[unit]` **P0**
- Claims carry username and is_admin correctly `[unit]` **P0**
- 7-day default expiry `[unit]` **P1**

---

## SSH Key Validation (`auth/ssh.rs`)

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
