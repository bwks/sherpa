-- Schema version 5: Generation sections by pull request. Which pull request counts a task or a
-- response of the host is worked out when reported (`generation::placement`); only what a
-- person named is stored. See "Which pull request counts what" in
-- docs/architecture/AGENT_BUS.md.

-- The pull request a focus named (`work use --pr N`, `send --pr N`); NULL when it named an
-- issue or a branch, and on rows recorded before.
ALTER TABLE host_focus ADD COLUMN pr_repo TEXT;
ALTER TABLE host_focus ADD COLUMN pr_number INTEGER;

-- The pull request `work reassign --to pr:N` pinned a response to; NULL otherwise.
ALTER TABLE host_usage ADD COLUMN pr_repo TEXT;
ALTER TABLE host_usage ADD COLUMN pr_number INTEGER;
