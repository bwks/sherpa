-- Whether the node's definition came from the startup import of a saved manifest. A saved
-- manifest held the node's `ztp.config` and `environment_variables` resolved, so the import
-- left them out, and the definition cannot rebuild the node as it was: the web UI refuses to
-- redeploy a marked node. sherpad marks every node of a lab that still has a saved manifest when
-- it imports or deletes it; a `sherpa redeploy` that sends the parts as written clears the
-- node's mark. Nodes stored before this version are not marked here: the startup step marks
-- those whose lab still has its saved manifest.
ALTER TABLE node ADD COLUMN definition_imported INTEGER NOT NULL DEFAULT 0
    CHECK (definition_imported IN (0, 1));
