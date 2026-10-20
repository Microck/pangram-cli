import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { releasePlatforms } from "./release-platforms.mjs";
import { platformPackagesPlugin } from "./platform-packages-plugin.mts";

test("all native package versions follow the wrapper before lockfile/PR hooks", async () => {
  const root = await mkdtemp(join(tmpdir(), "pangram-package-versions-"));
  try {
    await mkdir(join(root, "npm/pangram-cli"), { recursive: true });
    const dependencies = {};
    for (const { directory, os, cpu } of releasePlatforms) {
      const name = `@microck/${directory}`;
      dependencies[name] = "1.3.0";
      await mkdir(join(root, "npm", directory), { recursive: true });
      await writeFile(join(root, "npm", directory, "package.json"), JSON.stringify({ name, version: "1.3.0", os: [os], cpu: [cpu] }));
    }
    await writeFile(join(root, "npm/pangram-cli/package.json"), JSON.stringify({ version: "1.3.1", optionalDependencies: dependencies }));
    await platformPackagesPlugin(root).applyDraft();
    const main = JSON.parse(await readFile(join(root, "npm/pangram-cli/package.json")));
    for (const { directory, os, cpu } of releasePlatforms) {
      const manifest = JSON.parse(await readFile(join(root, "npm", directory, "package.json")));
      assert.equal(manifest.version, main.version);
      assert.equal(main.optionalDependencies[manifest.name], main.version);
      assert.deepEqual(manifest.os, [os]);
      assert.deepEqual(manifest.cpu, [cpu]);
    }

  } finally { await rm(root, { recursive: true, force: true }); }
});
