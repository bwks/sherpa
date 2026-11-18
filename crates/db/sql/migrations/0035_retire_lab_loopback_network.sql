-- A node's serial console is a UNIX socket only sherpad reaches (#674), no longer a telnet
-- listener on the node's address in its lab's loopback network, so a lab is given no loopback
-- network any more, and nothing reads the one recorded. The column cannot be dropped, as it is
-- UNIQUE, and `lab` cannot be rebuilt under the foreign keys that reference it, so it is renamed
-- to say it is unused. A new lab stores its own `lab_id` in it, which is unique and NOT NULL.
ALTER TABLE lab RENAME COLUMN loopback_network TO retired_loopback_network;
