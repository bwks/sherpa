-- Index 0 of every model's interface table is its management interface, so whether an image had a
-- dedicated one no longer changed anything.
ALTER TABLE node_image DROP COLUMN dedicated_management_interface;
