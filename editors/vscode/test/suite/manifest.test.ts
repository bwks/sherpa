import { describe, it, beforeEach, afterEach } from "node:test";
import * as assert from "node:assert/strict";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

import { getDevboxNodes } from "../../src/manifest";

let tmpDir: string;

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "sherpa-test-"));
});

afterEach(() => {
  fs.rmSync(tmpDir, { recursive: true });
});

function writeManifest(content: string): string {
  const p = path.join(tmpDir, "manifest.toml");
  fs.writeFileSync(p, content);
  return p;
}

describe("getDevboxNodes", () => {
  it("returns devbox_linux nodes", () => {
    const p = writeManifest(`
name = "test-lab"
nodes = [
  { name = "dev00", model = "devbox_linux" },
  { name = "dev01", model = "arista_veos" },
  { name = "dev02", model = "devbox_linux" },
]
`);
    const nodes = getDevboxNodes(p);
    assert.equal(nodes.length, 2);
    assert.equal(nodes[0].name, "dev00");
    assert.equal(nodes[1].name, "dev02");
  });

  it("returns empty array when no devbox nodes", () => {
    const p = writeManifest(`
name = "test-lab"
nodes = [
  { name = "dev01", model = "arista_veos" },
]
`);
    const nodes = getDevboxNodes(p);
    assert.equal(nodes.length, 0);
  });

  it("returns empty array when no nodes key", () => {
    const p = writeManifest(`
name = "test-lab"
`);
    const nodes = getDevboxNodes(p);
    assert.equal(nodes.length, 0);
  });

  it("handles multiple node fields", () => {
    const p = writeManifest(`
name = "test-lab"
nodes = [
  { name = "dev00", model = "devbox_linux", boot_disk_size = 100, cpu_count = 4 },
]
`);
    const nodes = getDevboxNodes(p);
    assert.equal(nodes.length, 1);
    assert.equal(nodes[0].name, "dev00");
    assert.equal(nodes[0].model, "devbox_linux");
  });
});
