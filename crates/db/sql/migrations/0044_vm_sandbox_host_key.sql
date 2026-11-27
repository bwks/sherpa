-- The SSH host key the sandbox's own sshd offers on its own address, recorded once it answers and
-- listed in the lab's known hosts file. `down` and `resume` keep it; `redeploy` clears it and
-- records the new VM's.
ALTER TABLE vm_sandbox ADD COLUMN ssh_host_key TEXT;
