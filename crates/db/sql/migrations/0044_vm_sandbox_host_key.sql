-- Add the sandbox's SSH host key to vm_sandbox.
ALTER TABLE vm_sandbox ADD COLUMN ssh_host_key TEXT;
