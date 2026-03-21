# Server TLS — Test Specifications

> **Crate:** `crates/server/` (`tls/`)
> **External Dependencies:** Filesystem for cert storage
> **Existing Tests:** None

---

## Certificate Generation

**What to test:**
- Self-signed certificate generated successfully `[unit]` **P0**
- Generated cert is valid X.509 `[unit]` **P0**
- Certificate contains correct subject/issuer information `[unit]` **P1**
- Private key generated alongside certificate `[unit]` **P0**
- Files written to correct paths `[integration]` **P1**

---

## Certificate Loading

**What to test:**
- Valid cert and key loaded from disk `[integration]` **P0**
- Invalid/corrupt cert file produces error `[integration]` **P0**
- Missing cert file produces error `[integration]` **P0**
- Mismatched cert/key pair detected `[integration]` **P1**

---

## Certificate Manager

**What to test:**
- Auto-generates cert if none exists on startup `[integration]` **P0**
- Uses existing cert if already present `[integration]` **P0**
- Cert served via /cert endpoint in PEM format `[integration]` **P0**
- Renewal/rotation when cert expires `[integration]` **P2**
