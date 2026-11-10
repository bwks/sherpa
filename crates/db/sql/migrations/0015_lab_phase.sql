-- The lab lifecycle is explicit. `phase` says which operation owns a lab's record:
-- `creating` while `up` builds it, `destroying` while a destroy removes it, `destroy_failed` when
-- a destroy left something behind, `interrupted` when the server stopped during `up`, and
-- `ready` otherwise. Every lab that exists before this version was built, so it is `ready`.
-- The status is no longer stored: it is derived from the lab's nodes when read.
ALTER TABLE lab ADD COLUMN phase TEXT NOT NULL DEFAULT 'ready';
ALTER TABLE lab DROP COLUMN status;
