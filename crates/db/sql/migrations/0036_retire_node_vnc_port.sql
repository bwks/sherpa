-- A VM's VNC console is a UNIX socket only sherpad reaches (#674), no longer a TCP port on the
-- server, so no node is given a VNC port any more and nothing reads the one recorded. The column
-- cannot be dropped, as it is UNIQUE, and `node` cannot be rebuilt under the foreign keys that
-- reference it, so its values are cleared and it is renamed to say it is unused. It stays NULL.
UPDATE node SET vnc_port = NULL;
ALTER TABLE node RENAME COLUMN vnc_port TO retired_vnc_port;
