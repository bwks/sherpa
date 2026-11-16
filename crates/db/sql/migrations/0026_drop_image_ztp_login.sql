-- A node image no longer records a ZTP username, password or password-auth setting: Sherpa's
-- generated node configurations never read them (#649).
-- Any values stored, the sealed password included, are dropped with the columns, and so are the
-- two triggers that refused a password not sealed. The image's ZTP setting, `ztp`, stays.
DROP TRIGGER IF EXISTS node_image_ztp_password_sealed_on_insert;
DROP TRIGGER IF EXISTS node_image_ztp_password_sealed_on_update;
ALTER TABLE node_image DROP COLUMN ztp_username;
ALTER TABLE node_image DROP COLUMN ztp_password;
ALTER TABLE node_image DROP COLUMN ztp_password_auth;
