import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";

import { MANIFEST_FILE, SSH_CONFIG_FILE } from "./constants";
import { getDevboxNodes, ManifestNode } from "./manifest";
import { createSshConfigWatcher, parseHostNames } from "./sshConfigWatcher";
import { connectToDevbox } from "./remoteConnect";

let watcher: vscode.FileSystemWatcher | undefined;

export function activate(context: vscode.ExtensionContext): void {
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
  if (!workspaceFolder) {
    return;
  }

  const manifestPath = path.join(workspaceFolder.uri.fsPath, MANIFEST_FILE);
  if (!fs.existsSync(manifestPath)) {
    return;
  }

  let devboxNodes: ManifestNode[];
  try {
    devboxNodes = getDevboxNodes(manifestPath);
  } catch {
    return;
  }

  if (devboxNodes.length === 0) {
    return;
  }

  // Register the manual connect command
  const connectCmd = vscode.commands.registerCommand(
    "sherpa.connectDevbox",
    () => handleConnect(workspaceFolder, devboxNodes)
  );
  context.subscriptions.push(connectCmd);

  // Check if sherpa_ssh_config already exists
  const sshConfigPath = path.join(
    workspaceFolder.uri.fsPath,
    SSH_CONFIG_FILE
  );
  if (fs.existsSync(sshConfigPath)) {
    onSshConfigDetected(workspaceFolder, devboxNodes);
  }

  // Watch for sherpa_ssh_config changes
  watcher = createSshConfigWatcher(workspaceFolder);

  watcher.onDidCreate(() => {
    onSshConfigDetected(workspaceFolder, devboxNodes);
  });

  watcher.onDidChange(() => {
    onSshConfigDetected(workspaceFolder, devboxNodes);
  });

  watcher.onDidDelete(() => {
    vscode.window.showInformationMessage("Sherpa SSH config removed.");
  });

  context.subscriptions.push(watcher);
}

async function onSshConfigDetected(
  workspaceFolder: vscode.WorkspaceFolder,
  devboxNodes: ManifestNode[]
): Promise<void> {
  const config = vscode.workspace.getConfiguration("sherpa");
  if (!config.get<boolean>("autoConnect", true)) {
    return;
  }

  await handleConnect(workspaceFolder, devboxNodes);
}

async function handleConnect(
  workspaceFolder: vscode.WorkspaceFolder,
  devboxNodes: ManifestNode[]
): Promise<void> {
  const sshConfigPath = path.join(
    workspaceFolder.uri.fsPath,
    SSH_CONFIG_FILE
  );
  if (!fs.existsSync(sshConfigPath)) {
    vscode.window.showWarningMessage(
      "No sherpa_ssh_config found. Run 'sherpa up' first."
    );
    return;
  }

  // Parse available hosts from the SSH config
  const availableHosts = parseHostNames(sshConfigPath);
  const devboxNodeNames = new Set(devboxNodes.map((n) => n.name));

  // Filter hosts to only devbox nodes (match by node name prefix before the dot)
  const devboxHosts = availableHosts.filter((host) => {
    const nodeName = host.split(".")[0];
    return devboxNodeNames.has(nodeName);
  });

  if (devboxHosts.length === 0) {
    vscode.window.showWarningMessage("No devbox hosts found in SSH config.");
    return;
  }

  const remotePath = vscode.workspace
    .getConfiguration("sherpa")
    .get<string>("remotePath", "/home/sherpa");

  if (devboxHosts.length === 1) {
    const choice = await vscode.window.showInformationMessage(
      `Sherpa devbox ready: ${devboxHosts[0]}`,
      "Connect",
      "Dismiss"
    );
    if (choice === "Connect") {
      await connectToDevbox(devboxHosts[0], remotePath);
    }
  } else {
    const selected = await vscode.window.showQuickPick(devboxHosts, {
      placeHolder: "Select a devbox to connect to",
    });
    if (selected) {
      await connectToDevbox(selected, remotePath);
    }
  }
}

export function deactivate(): void {
  watcher?.dispose();
}
