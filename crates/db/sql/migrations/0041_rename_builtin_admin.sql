-- The built-in admin account is renamed `sherpa-admin` (#895). The row keeps its id, so its
-- password, SSH keys, lock state, sessions and labs stay with it. The update trigger refuses
-- this rename, so the protecting triggers are dropped, the row renamed, and the triggers made
-- again for the new name. sherpad refuses this migration while a `sherpa-admin` user exists.
-- The message is ADMIN_PROTECTED_MESSAGE.
DROP TRIGGER IF EXISTS user_protect_admin_insert;
DROP TRIGGER IF EXISTS user_protect_admin_update;
DROP TRIGGER IF EXISTS user_protect_admin_delete;
UPDATE user SET username = 'sherpa-admin' WHERE username = 'admin';
CREATE TRIGGER user_protect_admin_insert BEFORE INSERT ON user
    WHEN NEW.username = 'sherpa-admin' AND NEW.is_admin != 1
    BEGIN SELECT RAISE(ABORT, 'The built-in sherpa-admin account cannot be deleted, renamed or lose administrator rights'); END;
CREATE TRIGGER user_protect_admin_update BEFORE UPDATE ON user
    WHEN (OLD.username = 'sherpa-admin' AND (NEW.username != 'sherpa-admin' OR NEW.is_admin != 1))
        OR (NEW.username = 'sherpa-admin' AND NEW.is_admin != 1)
    BEGIN SELECT RAISE(ABORT, 'The built-in sherpa-admin account cannot be deleted, renamed or lose administrator rights'); END;
CREATE TRIGGER user_protect_admin_delete BEFORE DELETE ON user
    WHEN OLD.username = 'sherpa-admin'
    BEGIN SELECT RAISE(ABORT, 'The built-in sherpa-admin account cannot be deleted, renamed or lose administrator rights'); END;
