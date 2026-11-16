-- The startup import of saved manifests is gone, so nothing marks a node as imported any more,
-- and the web UI no longer refuses to redeploy one that was. A node imported by 0.6.4 or 0.6.5
-- and never redeployed from the CLI since has no `ztp.config`, environment variables or
-- provisioners in its definition; a web redeploy rebuilds it without them. The upgrade notes
-- for 0.7.0 say to redeploy such nodes from the CLI, or destroy their lab, first.
ALTER TABLE node DROP COLUMN definition_imported;
