import { describe, it, beforeEach, afterEach } from "node:test";
import * as assert from "node:assert/strict";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

import { parseHostNames } from "../../src/sshParser";

let tmpDir: string;

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "sherpa-test-"));
});

afterEach(() => {
  fs.rmSync(tmpDir, { recursive: true });
});

function writeSshConfig(content: string): string {
  const p = path.join(tmpDir, "sherpa_ssh_config");
  fs.writeFileSync(p, content);
  return p;
}

describe("parseHostNames", () => {
  it("parses scoped host names from ssh config", () => {
    const p = writeSshConfig(`
Host 172.20.0.10 dev00.abcd1234 dev00.abcd1234.sherpa.lab.local
    HostName 172.20.0.10
    Port 22
    User sherpa

Host 172.20.0.11 dev01.abcd1234 dev01.abcd1234.sherpa.lab.local
    HostName 172.20.0.11
    Port 22
    User sherpa
`);
    const hosts = parseHostNames(p);
    assert.deepEqual(hosts, ["dev00.abcd1234", "dev01.abcd1234"]);
  });

  it("returns empty array for empty config", () => {
    const p = writeSshConfig("");
    const hosts = parseHostNames(p);
    assert.deepEqual(hosts, []);
  });

  it("returns empty array for config with no Host lines", () => {
    const p = writeSshConfig(`
    HostName 172.20.0.10
    Port 22
    User sherpa
`);
    const hosts = parseHostNames(p);
    assert.deepEqual(hosts, []);
  });

  it("handles single host entry", () => {
    const p = writeSshConfig(
      "Host 172.20.0.10 dev00.abcd1234 dev00.abcd1234.sherpa.lab.local"
    );
    const hosts = parseHostNames(p);
    assert.deepEqual(hosts, ["dev00.abcd1234"]);
  });

  it("skips Host lines with fewer than 3 parts", () => {
    const p = writeSshConfig(`
Host 172.20.0.10
    HostName 172.20.0.10

Host 172.20.0.11 dev01.abcd1234 dev01.abcd1234.sherpa.lab.local
    HostName 172.20.0.11
`);
    const hosts = parseHostNames(p);
    assert.deepEqual(hosts, ["dev01.abcd1234"]);
  });
});
