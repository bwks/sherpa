-- The login limit's counts (#756): one row per username and address pair, per address and per
-- username that has attempts counted against it, so a lock outlives a restart of sherpad.
-- `username` is empty on an address row and `address` on a username row. `address` is an IPv4
-- address or an IPv6 /64 prefix such as `2001:db8::/64`. Times are Unix seconds: `window_start`
-- is the row's first counted attempt and `locked_until` when its lock ends, NULL while it is not
-- locked. No password, nor anything made from one, is stored. A periodic sweep in sherpad
-- deletes the rows that limit nothing any more.
CREATE TABLE login_limit (
    kind TEXT NOT NULL CHECK (kind IN ('pair', 'address', 'username')),
    username TEXT NOT NULL,
    address TEXT NOT NULL,
    failures INTEGER NOT NULL CHECK (failures >= 0),
    window_start INTEGER NOT NULL,
    locked_until INTEGER,
    PRIMARY KEY (kind, username, address),
    CHECK ((kind = 'pair' AND address != '')
        OR (kind = 'address' AND username = '' AND address != '')
        OR (kind = 'username' AND address = ''))
) STRICT;
-- Not led by `kind`: an index that is would be taken over the primary key for a lookup by key.
CREATE INDEX login_limit_window_start ON login_limit (window_start);

-- How many rows of each kind `login_limit` holds, kept by its triggers, so the cap on each kind
-- is checked without counting the rows.
CREATE TABLE login_limit_size (
    kind TEXT PRIMARY KEY CHECK (kind IN ('pair', 'address', 'username')),
    keys INTEGER NOT NULL CHECK (keys >= 0)
) STRICT;
INSERT INTO login_limit_size (kind, keys) VALUES ('pair', 0), ('address', 0), ('username', 0);
CREATE TRIGGER login_limit_size_insert AFTER INSERT ON login_limit
    BEGIN UPDATE login_limit_size SET keys = keys + 1 WHERE kind = NEW.kind; END;
CREATE TRIGGER login_limit_size_delete AFTER DELETE ON login_limit
    BEGIN UPDATE login_limit_size SET keys = keys - 1 WHERE kind = OLD.kind; END;
