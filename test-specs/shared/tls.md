# Shared TLS Utilities — Test Specifications

> **Crate:** `crates/shared/` (`tls/`)
> **External Dependencies:** Filesystem (~/.sherpa/trusted_certs/), network (cert fetch)
> **Existing Tests:** Inline tests in trust_store.rs

---

## Certificate Fetching (`tls/cert_fetch.rs`)

**What to test:**
- `fetch_server_certificate()` retrieves cert from /cert HTTP endpoint `[integration]` **P0**
- Fetched cert is valid PEM format `[integration]` **P0**
- 5-second timeout on fetch `[integration]` **P1**
- Server URL parsing extracts host:port correctly `[unit]` **P0**
- Invalid server URL rejected `[unit]` **P0**
- Unreachable server produces clear error `[integration]` **P0**

---

## TLS Configuration (`tls/config.rs`)

**What to test:**
- Build with custom CA cert path loads cert correctly `[unit]` **P0**
- Build with system certs uses OS trust store `[unit]` **P1**
- Build with skip_verify disables certificate validation `[unit]` **P1**
- Trust-on-first-use flow: check trust store → fetch if missing → save `[integration]` **P0**
- Build with invalid cert path produces error `[unit]` **P0**

---

## Trust Store (`tls/trust_store.rs`)

**What to test:**
- `TrustStore::new()` creates directory with 0o700 permissions `[unit]` **P0**
- `save_cert()` writes cert file with 0o600 permissions `[unit]` **P0**
- `get_cert()` returns saved cert for known server `[unit]` **P0**
- `get_cert()` returns None for unknown server `[unit]` **P0**
- `remove_cert()` deletes cert, returns true `[unit]` **P0**
- `remove_cert()` returns false if cert didn't exist `[unit]` **P0**
- `list_all()` returns all (server_url, cert_pem) tuples `[unit]` **P1**
- Server URL sanitized for use as filename `[unit]` **P1**

---

## Certificate Info Extraction

**What to test:**
- `extract_cert_info()` parses subject, issuer, validity dates `[unit]` **P0**
- `compute_fingerprint()` produces SHA-256 colon-separated hex `[unit]` **P0**
- Invalid PEM produces error `[unit]` **P0**

**Existing coverage:** Inline tests in trust_store.rs cover basic operations
