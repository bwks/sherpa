-- Login sessions (#501). Every token names a row here by its `sid` claim, and is accepted only
-- while the row exists and `expires_at` (Unix seconds, the token's `exp`) has not passed.
-- Logout deletes one row, a password change deletes the user's other rows, deleting the user
-- deletes all of them (ON DELETE CASCADE), and a periodic sweep deletes expired rows.
-- Tokens issued before this version have no `sid`, so every user logs in again after upgrading.
CREATE TABLE session (
    id TEXT PRIMARY KEY,
    user TEXT NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at INTEGER NOT NULL
) STRICT;
CREATE INDEX session_user ON session (user);
CREATE INDEX session_expires_at ON session (expires_at);
