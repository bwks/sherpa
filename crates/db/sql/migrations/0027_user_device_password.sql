-- A user's device password, their login password on lab nodes (#649). It is a secret, so the
-- column holds only sealed text: `sealed:` and the sealed value in lower-case hex, bound to the
-- user's row (see `crate::secret`). The triggers refuse anything else, so plain text cannot be
-- written. 65 characters is the prefix and the hex of the shortest sealed value, 29 bytes.
-- NULL is no device password; every user starts without one.
ALTER TABLE user ADD COLUMN device_password TEXT;
CREATE TRIGGER user_device_password_sealed_on_insert BEFORE INSERT ON user
    WHEN NEW.device_password IS NOT NULL
        AND NOT (NEW.device_password GLOB 'sealed:*'
            AND length(NEW.device_password) >= 65
            AND substr(NEW.device_password, 8) NOT GLOB '*[^0-9a-f]*')
    BEGIN SELECT RAISE(ABORT, 'user.device_password must be sealed'); END;
CREATE TRIGGER user_device_password_sealed_on_update BEFORE UPDATE OF device_password ON user
    WHEN NEW.device_password IS NOT NULL
        AND NOT (NEW.device_password GLOB 'sealed:*'
            AND length(NEW.device_password) >= 65
            AND substr(NEW.device_password, 8) NOT GLOB '*[^0-9a-f]*')
    BEGIN SELECT RAISE(ABORT, 'user.device_password must be sealed'); END;
