import { execFileSync } from "node:child_process";
import { chmod, copyFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { releasePlatforms } from "./release-platforms.mjs";

const artifacts = resolve(process.argv[2] ?? "release-artifacts");
const artworkNotice = resolve("assets/brand/NOTICE.md");
const version = process.argv[3];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  throw new Error("usage: node scripts/stage-npm-packages.mjs ARTIFACTS VERSION");
}

for (const { target, directory, executable, extension } of releasePlatforms) {
  const archive = join(artifacts, `pangram-v${version}-${target}.${extension}`);
  const staging = await mkdtemp(join(tmpdir(), "pangram-npm-"));
  try {
    if (extension === "zip") {
      execFileSync("unzip", ["-q", archive, executable, "-d", staging]);
    } else {
      execFileSync("tar", ["-xJf", archive, "-C", staging, executable]);
    }
    const destinationDirectory = resolve("npm", directory, "bin");
    await mkdir(destinationDirectory, { recursive: true });
    const destination = join(destinationDirectory, executable);
    await copyFile(join(staging, executable), destination);
    await copyFile(artworkNotice, resolve("npm", directory, "NOTICE.md"));
    if (executable === "pangram") await chmod(destination, 0o755);
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

await copyFile(artworkNotice, resolve("npm/pangram-cli/NOTICE.md"));
