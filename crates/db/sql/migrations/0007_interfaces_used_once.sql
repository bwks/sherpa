-- A node interface is in at most one link or one bridge, once. Only new rows are checked, so a
-- database that already holds a repeat still opens.
CREATE TRIGGER link_interfaces_used_once BEFORE INSERT ON link
    WHEN (NEW.node_a = NEW.node_b AND NEW.int_a = NEW.int_b)
        OR EXISTS (
            SELECT 1 FROM link
            WHERE (node_a = NEW.node_a AND int_a = NEW.int_a)
                OR (node_b = NEW.node_a AND int_b = NEW.int_a)
                OR (node_a = NEW.node_b AND int_a = NEW.int_b)
                OR (node_b = NEW.node_b AND int_b = NEW.int_b)
        )
        OR EXISTS (
            SELECT 1 FROM bridge_member
            WHERE (node = NEW.node_a AND interface = NEW.int_a)
                OR (node = NEW.node_b AND interface = NEW.int_b)
        )
    BEGIN SELECT RAISE(ABORT, 'Interface already in use by another link or bridge'); END;
CREATE TRIGGER bridge_member_interface_used_once BEFORE INSERT ON bridge_member
    WHEN EXISTS (
            SELECT 1 FROM bridge_member WHERE node = NEW.node AND interface = NEW.interface
        )
        OR EXISTS (
            SELECT 1 FROM link
            WHERE (node_a = NEW.node AND int_a = NEW.interface)
                OR (node_b = NEW.node AND int_b = NEW.interface)
        )
    BEGIN SELECT RAISE(ABORT, 'Interface already in use by another link or bridge'); END;
