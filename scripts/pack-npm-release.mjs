import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { basename, resolve } from "node:path";
import { releasePlatforms } from "./release-platforms.mjs";

const destination = resolve(process.argv[2] ?? "release-verification/npm");
const version = process.argv[3];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  throw new Error("usage: node scripts/pack-npm-release.mjs DESTINATION VERSION");
}

await mkdir(destination, { recursive: true });

function pack(directory) {
  const output = execFileSync(
    "npm",
    [
      "pack",
      "--ignore-scripts",
      "--json",
      "--pack-destination",
      destination,
      resolve("npm", directory),
    ],
    { encoding: "utf8" },
  );
  const packed = JSON.parse(output);
  if (packed.length !== 1 || typeof packed[0]?.filename !== "string") {
    throw new Error(`npm returned an unexpected pack result for ${directory}`);
  }
  return basename(packed[0].filename);
}

const targets = Object.fromEntries(
  releasePlatforms.map(({ target, directory }) => [target, pack(directory)]),
);
const packages = {
  version,
  wrapper: pack("pangram-cli"),
  targets,
};

await writeFile(
  resolve(destination, "packages.json"),
  `${JSON.stringify(packages, null, 2)}\n`,
  "utf8",
);
