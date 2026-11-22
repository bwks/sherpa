-- Account lockout. `failed_logins` counts the logins started since the account's last
-- successful one; the one that takes it to the limit locks the account, setting `locked_at`
-- (RFC 3339 text), and a locked account is refused until an admin unlocks it, which sets
-- both back. NULL is not locked.
ALTER TABLE user ADD COLUMN failed_logins INTEGER NOT NULL DEFAULT 0 CHECK (failed_logins >= 0);
ALTER TABLE user ADD COLUMN locked_at TEXT;
-- Every login attempt on an account that is not locked: when, whether it succeeded, and the
-- address it came from as 4 bytes for IPv4 or 16 for IPv6. Rows go only with their user.
CREATE TABLE login_attempt (
    id TEXT PRIMARY KEY,
    user TEXT NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    attempted_at TEXT NOT NULL,
    succeeded INTEGER NOT NULL CHECK (succeeded IN (0, 1)),
    address BLOB NOT NULL CHECK (length(address) = 4 OR length(address) = 16)
) STRICT;
CREATE INDEX login_attempt_user_attempted_at ON login_attempt (user, attempted_at);
