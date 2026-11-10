-- The node lifecycle is explicit: operations and the scanner move a node only between the
-- states it allows. `created` was the scanner's word for a container Docker had created and not
-- started; such a node is stopped. Nodes are now inserted as `building`, and a node with no
-- domain or container is `missing`. Unknown text still decodes as `unknown`.
UPDATE node SET state = 'stopped' WHERE state = 'created';
