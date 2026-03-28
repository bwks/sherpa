import * as fs from "fs";

/**
 * Parse host aliases from a sherpa_ssh_config file.
 * Returns the second alias from each Host line (the <node>.<lab-id> form).
 */
export function parseHostNames(sshConfigPath: string): string[] {
  const content = fs.readFileSync(sshConfigPath, "utf-8");
  const hosts: string[] = [];

  for (const line of content.split("\n")) {
    const trimmed = line.trim();
    if (trimmed.startsWith("Host ")) {
      // Host <ip> <node>.<lab-id> <node>.<lab-id>.<domain>
      const parts = trimmed.split(/\s+/);
      if (parts.length >= 3) {
        hosts.push(parts[2]);
      }
    }
  }

  return hosts;
}
