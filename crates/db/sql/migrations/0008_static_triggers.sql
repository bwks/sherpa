-- Enum columns are no longer checked by triggers: every value sherpad writes is serialized from
-- its Rust enum, and every value it reads is decoded back, so a value outside the enum fails when
-- it is read, naming the row. The triggers were rebuilt from the enums on every start, which made
-- a row holding a newer release's value impossible to write after a downgrade.
DROP TRIGGER IF EXISTS node_image_rules_insert;
DROP TRIGGER IF EXISTS node_image_rules_update;
DROP TRIGGER IF EXISTS lab_rules_insert;
DROP TRIGGER IF EXISTS lab_rules_update;
DROP TRIGGER IF EXISTS node_rules_insert;
DROP TRIGGER IF EXISTS node_rules_update;
DROP TRIGGER IF EXISTS link_rules_insert;
DROP TRIGGER IF EXISTS link_rules_update;

-- The built-in admin account cannot be deleted, renamed or lose its administrator flag, and no
-- admin row can exist without the flag. These were also recreated on every start; they are fixed
-- text, so they belong here. The message is ADMIN_PROTECTED_MESSAGE.
DROP TRIGGER IF EXISTS user_protect_admin_insert;
CREATE TRIGGER user_protect_admin_insert BEFORE INSERT ON user
    WHEN NEW.username = 'admin' AND NEW.is_admin != 1
    BEGIN SELECT RAISE(ABORT, 'The built-in admin account cannot be deleted, renamed or lose administrator rights'); END;
DROP TRIGGER IF EXISTS user_protect_admin_update;
CREATE TRIGGER user_protect_admin_update BEFORE UPDATE ON user
    WHEN (OLD.username = 'admin' AND (NEW.username != 'admin' OR NEW.is_admin != 1))
        OR (NEW.username = 'admin' AND NEW.is_admin != 1)
    BEGIN SELECT RAISE(ABORT, 'The built-in admin account cannot be deleted, renamed or lose administrator rights'); END;
DROP TRIGGER IF EXISTS user_protect_admin_delete;
CREATE TRIGGER user_protect_admin_delete BEFORE DELETE ON user
    WHEN OLD.username = 'admin'
    BEGIN SELECT RAISE(ABORT, 'The built-in admin account cannot be deleted, renamed or lose administrator rights'); END;
