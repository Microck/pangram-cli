import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { releasePlatforms } from "./release-platforms.mjs";

export async function syncPlatformPackageVersions(root = process.cwd()) {
  const mainPath = join(root, "npm/pangram-cli/package.json");
  const main = JSON.parse(await readFile(mainPath, "utf8"));
  for (const { directory } of releasePlatforms) {
    const path = join(root, `npm/${directory}/package.json`);
    const manifest = JSON.parse(await readFile(path, "utf8"));
    manifest.version = main.version;
    main.optionalDependencies[manifest.name] = main.version;
    await writeFile(path, `${JSON.stringify(manifest, null, 2)}\n`);
  }
  await writeFile(mainPath, `${JSON.stringify(main, null, 2)}\n`);
}
