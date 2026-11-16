-- The manifest setting `provision_current_user` is gone (#649): every node with a generated
-- login gets the lab owner's account, so there is nothing left to choose. Stored definitions lose
-- the value with the column, and web redeploys of existing labs keep working.
ALTER TABLE lab DROP COLUMN provision_current_user;
