-- The login limit's failed logins (#756), one row per failure, so a lock outlives a restart of
-- sherpad. A lock is not stored: the limiter counts each key's rows over the last 15 minutes.
-- `address` is 4 bytes, an IPv4 address, or 8, the /64 prefix of an IPv6 address. `failed_at` is
-- Unix seconds. No password, nor anything made from one, is stored. A periodic sweep in sherpad
-- deletes the rows older than the window.
CREATE TABLE login_failure (
    username TEXT NOT NULL,
    address BLOB NOT NULL CHECK (length(address) IN (4, 8)),
    failed_at INTEGER NOT NULL
) STRICT;
CREATE INDEX login_failure_pair ON login_failure (username, address, failed_at);
CREATE INDEX login_failure_address ON login_failure (address, failed_at);
CREATE INDEX login_failure_username ON login_failure (username, failed_at);
