# Sherpa Remote - VS Code Extension

VS Code extension that auto-connects to Sherpa devbox VMs via Remote-SSH.

When you open a workspace containing a `manifest.toml` with `devbox_linux` nodes, the extension watches for the `sherpa_ssh_config` file created by `sherpa up` and offers to connect you directly to the devbox.

## Prerequisites

- [Remote-SSH](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-ssh) extension installed in VS Code
- Sherpa CLI installed and configured

## How It Works

1. Open a folder containing a `manifest.toml` in VS Code
2. Run `sherpa up` from the terminal to create your lab
3. The extension detects the `sherpa_ssh_config` file and prompts you to connect
4. Select a devbox node and VS Code opens a Remote-SSH session into it

The extension injects an `Include` line into `~/.ssh/config` pointing at the workspace's `sherpa_ssh_config`. This allows Remote-SSH to resolve the devbox hostnames. The Include block is scoped by workspace path and automatically cleaned up when `sherpa_ssh_config` is deleted (e.g. by `sherpa destroy`).

## Commands

- **Sherpa: Connect to Devbox** - Manually trigger the devbox connection picker

## Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `sherpa.autoConnect` | `true` | Automatically prompt to connect when `sherpa_ssh_config` is detected |
| `sherpa.remoteUser` | `sherpa` | SSH user on the devbox |
| `sherpa.remotePath` | `/home/sherpa` | Remote folder to open on the devbox |

## Building

Requires Node.js >= 20.

```bash
cd editors/vscode
npm install
npm run compile
```

## Packaging

```bash
npm install -g @vscode/vsce
vsce package
```

This produces a `sherpa-remote-<version>.vsix` file.

## Installing

```bash
code --install-extension sherpa-remote-<version>.vsix
```

## Development

1. Open `editors/vscode/` in VS Code
2. Run `npm install` and `npm run compile`
3. Press `F5` to launch the Extension Development Host
4. Open a folder with a `manifest.toml` and `sherpa_ssh_config` in the dev host window
