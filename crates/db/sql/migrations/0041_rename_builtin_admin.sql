-- Rename the built-in `admin` account to `sherpa-admin` and recreate its protecting triggers.
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
