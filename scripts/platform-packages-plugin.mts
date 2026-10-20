import type { TegamiPlugin } from "tegami";
import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import { promisify } from "node:util";
import { releasePlatforms } from "./release-platforms.mjs";
import { syncPlatformPackageVersions } from "./sync-platform-package-versions.mjs";

const run = promisify(execFile);

export function platformPackagesPlugin(root = process.cwd()): TegamiPlugin {
  return {
    name: "pangram-platform-packages",
    enforce: "post",
    async applyDraft() {
      // Run after npm's version writes, but before applyCliDraft updates the
      // lockfile and GitHub snapshots the version branch.
      await syncPlatformPackageVersions(root);
    },
    async willPublish({ pkg }) {
      if (pkg.name !== "@microck/pangram-cli") return;
      for (const { directory } of releasePlatforms) {
        const path = `npm/${directory}`;
        const manifest = JSON.parse(await readFile(`${path}/package.json`, "utf8"));
        // `npm view` can exit 0 with empty output for a version that does
        // not exist, so only an exact version echo counts as published.
        try {
          const { stdout } = await run("npm", [
            "view",
            `${manifest.name}@${manifest.version}`,
            "version",
          ]);
          if (stdout.trim() === manifest.version) continue;
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

