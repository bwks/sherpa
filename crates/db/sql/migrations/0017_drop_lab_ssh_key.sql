-- Every lab has its own key pair in its lab folder, and the server's shared key is gone, so the
-- key a lab uses no longer needs recording. `apply_schema` refuses this migration while any lab
-- is still `server`, naming them, as their nodes would lose SSH access.
ALTER TABLE lab DROP COLUMN ssh_key;
