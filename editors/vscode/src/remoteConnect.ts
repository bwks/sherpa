import * as vscode from "vscode";

/**
 * Open a remote SSH folder in a new VS Code window.
 */
export async function connectToDevbox(
  hostName: string,
  remotePath: string
): Promise<void> {
  const uri = vscode.Uri.parse(
    `vscode-remote://ssh-remote+${hostName}${remotePath}`
  );
  await vscode.commands.executeCommand("vscode.openFolder", uri, {
    forceNewWindow: false,
  });
}

/**
 * Check if the Remote-SSH extension is installed.
 * Note: In the Extension Development Host, other extensions may not be loaded.
 * This check is best-effort.
 */
export function isRemoteSshInstalled(): boolean {
  return (
    vscode.extensions.getExtension("ms-vscode-remote.remote-ssh") !== undefined
  );
}
