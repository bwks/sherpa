# Shared Auth Utilities — Test Specifications

> **Crate:** `crates/shared/` (`auth/`)
> **External Dependencies:** None (pure crypto)
> **Existing Tests:** Inline tests in jwt.rs, password.rs
>
> Password strength and SSH key validation live in `validators/`; see `validators.md`.

---

## Password Hashing (`auth/password.rs`)

**What to test:**
- `hash_password()` produces Argon2id PHC format hash `[unit]` **P0**
- `verify_password()` returns true for correct password `[unit]` **P0**
- `verify_password()` returns false for incorrect password `[unit]` **P0**
- Different calls with same password produce different hashes (random salt) `[unit]` **P0**
- `hash_password()` rejects passwords that fail `validators::validate_password_strength()` `[unit]` **P0**
- Constant-time comparison used in verification `[unit]` **P2**

---

## JWT Operations (`auth/jwt.rs`)

**What to test:**
- `Claims::new()` sets correct iat (now) and exp (now + expiry_seconds) `[unit]` **P0**
- `Claims::is_expired()` returns false for fresh token `[unit]` **P0**
- `Claims::is_expired()` returns true for past expiry `[unit]` **P0**
- Claims carry username and is_admin correctly `[unit]` **P0**
- 7-day default expiry `[unit]` **P1**
