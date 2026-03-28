import * as fs from "fs";
import * as TOML from "@iarna/toml";

import { DEVBOX_LINUX_MODEL } from "./constants";

export interface ManifestNode {
  name: string;
  model: string;
}

interface Manifest {
  name: string;
  nodes?: ManifestNode[];
}

/**
 * Parse a manifest.toml file and return devbox_linux nodes.
 */
export function getDevboxNodes(manifestPath: string): ManifestNode[] {
  const content = fs.readFileSync(manifestPath, "utf-8");
  const manifest = TOML.parse(content) as unknown as Manifest;

  if (!manifest.nodes || !Array.isArray(manifest.nodes)) {
    return [];
  }

  return manifest.nodes.filter((node) => node.model === DEVBOX_LINUX_MODEL);
}
