#!/usr/bin/env node
import { tegami } from "tegami";
import { runCli } from "tegami/cli";
import type { TegamiPlugin } from "tegami";
import { cargo } from "tegami/plugins/cargo";
import { github } from "tegami/plugins/github";
import { execFile } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { promisify } from "node:util";
import { releasePlatforms } from "./release-platforms.mjs";

const run = promisify(execFile);

function platformPackagesPlugin(): TegamiPlugin {
  return {
    name: "pangram-platform-packages",
    enforce: "post",
    async applyCliDraft() {
      const mainPath = "npm/pangram-cli/package.json";
      const main = JSON.parse(await readFile(mainPath, "utf8"));
      for (const { directory } of releasePlatforms) {
        const path = `npm/${directory}/package.json`;
        const manifest = JSON.parse(await readFile(path, "utf8"));
        manifest.version = main.version;
        main.optionalDependencies[manifest.name] = main.version;
        await writeFile(path, `${JSON.stringify(manifest, null, 2)}\n`);
      }
      await writeFile(mainPath, `${JSON.stringify(main, null, 2)}\n`);
    },
    async willPublish({ pkg }) {
      if (pkg.name !== "@microck/pangram-cli") return;
      for (const { directory } of releasePlatforms) {
        const path = `npm/${directory}`;
        const manifest = JSON.parse(await readFile(`${path}/package.json`, "utf8"));
        try {
          await run("npm", ["view", `${manifest.name}@${manifest.version}`, "version"]);
          continue;
        } catch (error) {
          const stderr =
            error && typeof error === "object" && "stderr" in error
              ? String(error.stderr)
              : "";
          if (!stderr.includes("E404")) throw error;
        }
        await run("npm", ["publish", "--access", "public", "--provenance"], {
          cwd: path,
        });
      }
    },
  };
}

const paper = tegami({
  plugins: [
    cargo(),
    github({
      repo: "Microck/pangram-cli",
      createTags: false,
      release: false,
      versionPr: { base: "main" },
    }),
    platformPackagesPlugin(),
  ],
  npm: {
    client: "npm",
    updateLockFile: true,
    trustedPublish: {
      provider: "github",
      workflow: "publish.yml",
    },
  },
});

await runCli(paper);
