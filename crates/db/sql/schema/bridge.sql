
CREATE TABLE bridge (
    id TEXT PRIMARY KEY,
    "index" INTEGER NOT NULL CHECK ("index" BETWEEN 0 AND 65535),
    bridge_name TEXT NOT NULL,
    network_name TEXT NOT NULL,
    lab TEXT NOT NULL REFERENCES lab(id) ON DELETE CASCADE,
    UNIQUE ("index", lab)
) STRICT;
CREATE INDEX bridge_lab ON bridge (lab);
CREATE TABLE bridge_member (
    bridge TEXT NOT NULL REFERENCES bridge(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    node TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    interface TEXT NOT NULL,
    PRIMARY KEY (bridge, position)
) STRICT;
CREATE INDEX bridge_member_node ON bridge_member (node);
