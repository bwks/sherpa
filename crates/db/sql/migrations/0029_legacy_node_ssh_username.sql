-- Nodes no longer get the shared `sherpa` account (#649): `up` and `redeploy` record the login
-- of every node, the lab owner's name or `admin` on Cisco FTDv, and a node with none recorded
-- logs in as the lab's owner. Nodes built before this version with no login recorded have only
-- `sherpa`, the published password and their lab's key, so they record `sherpa` and their SSH
-- config stays truthful until they are redeployed. sherpad warns at startup while any remain.
UPDATE node SET ssh_username = 'sherpa' WHERE ssh_username IS NULL;
