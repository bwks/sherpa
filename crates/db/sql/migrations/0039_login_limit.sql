-- The login limit's counts (#756), so a lock outlives a restart of sherpad: one table per key,
-- a username and address pair, an address for every username, and a username from every
-- address. `address` is 4 bytes, an IPv4 address, or 8, the /64 prefix of an IPv6 address.
-- Times are Unix seconds: `window_start` is the row's first counted attempt and `locked_until`
-- when its lock ends, NULL while it is not locked. No password, nor anything made from one, is
-- stored. A periodic sweep in sherpad deletes the rows that limit nothing any more.
CREATE TABLE login_limit_pair (
    username TEXT NOT NULL,
    address BLOB NOT NULL CHECK (length(address) IN (4, 8)),
    failures INTEGER NOT NULL CHECK (failures >= 0),
    window_start INTEGER NOT NULL,
    locked_until INTEGER,
    PRIMARY KEY (username, address)
) STRICT;
CREATE INDEX login_limit_pair_window_start ON login_limit_pair (window_start);

CREATE TABLE login_limit_address (
    address BLOB PRIMARY KEY CHECK (length(address) IN (4, 8)),
    failures INTEGER NOT NULL CHECK (failures >= 0),
    window_start INTEGER NOT NULL,
    locked_until INTEGER
) STRICT;
CREATE INDEX login_limit_address_window_start ON login_limit_address (window_start);

CREATE TABLE login_limit_username (
    username TEXT PRIMARY KEY,
    failures INTEGER NOT NULL CHECK (failures >= 0),
    window_start INTEGER NOT NULL,
    locked_until INTEGER
) STRICT;
CREATE INDEX login_limit_username_window_start ON login_limit_username (window_start);

-- How many rows each login limit table holds, in its one row, kept by the triggers below, so
-- the cap on each table is checked without counting its rows.
CREATE TABLE login_limit_size (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    pairs INTEGER NOT NULL CHECK (pairs >= 0),
    addresses INTEGER NOT NULL CHECK (addresses >= 0),
    usernames INTEGER NOT NULL CHECK (usernames >= 0)
) STRICT;
INSERT INTO login_limit_size (id, pairs, addresses, usernames) VALUES (1, 0, 0, 0);
CREATE TRIGGER login_limit_pair_insert AFTER INSERT ON login_limit_pair
    BEGIN UPDATE login_limit_size SET pairs = pairs + 1; END;
CREATE TRIGGER login_limit_pair_delete AFTER DELETE ON login_limit_pair
    BEGIN UPDATE login_limit_size SET pairs = pairs - 1; END;
CREATE TRIGGER login_limit_address_insert AFTER INSERT ON login_limit_address
    BEGIN UPDATE login_limit_size SET addresses = addresses + 1; END;
CREATE TRIGGER login_limit_address_delete AFTER DELETE ON login_limit_address
    BEGIN UPDATE login_limit_size SET addresses = addresses - 1; END;
CREATE TRIGGER login_limit_username_insert AFTER INSERT ON login_limit_username
    BEGIN UPDATE login_limit_size SET usernames = usernames + 1; END;
CREATE TRIGGER login_limit_username_delete AFTER DELETE ON login_limit_username
    BEGIN UPDATE login_limit_size SET usernames = usernames - 1; END;
