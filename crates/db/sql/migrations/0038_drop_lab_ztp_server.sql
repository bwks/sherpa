-- The manifest's `[ztp_server]` table is removed (#766): nothing read it, and node configs are
-- served without credentials. Its stored values, the sealed password included, are dropped with
-- the columns, and older labs' definitions still load.
ALTER TABLE lab DROP COLUMN ztp_server_enable;
ALTER TABLE lab DROP COLUMN ztp_server_username;
ALTER TABLE lab DROP COLUMN ztp_server_password;
