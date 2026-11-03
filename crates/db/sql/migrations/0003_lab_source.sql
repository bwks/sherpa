-- Where a lab was started from: the hostname of the machine `sherpa up` ran on, and the folder it
-- ran in. Labs started from the web UI or the API have neither.
ALTER TABLE lab ADD COLUMN client_host TEXT;
ALTER TABLE lab ADD COLUMN client_folder TEXT;
