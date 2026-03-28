import * as vscode from "vscode";

import { SSH_CONFIG_FILE } from "./constants";

export { parseHostNames } from "./sshParser";

/**
 * Create a FileSystemWatcher for sherpa_ssh_config in the given workspace folder.
 */
export function createSshConfigWatcher(
  workspaceFolder: vscode.WorkspaceFolder
): vscode.FileSystemWatcher {
  const pattern = new vscode.RelativePattern(workspaceFolder, SSH_CONFIG_FILE);
  return vscode.workspace.createFileSystemWatcher(pattern);
}
