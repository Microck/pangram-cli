import { readFile } from "node:fs/promises";
import { releasePlatforms } from "./release-platforms.mjs";

const main = JSON.parse(await readFile("npm/pangram-cli/package.json", "utf8"));
const wrapper = await readFile("npm/pangram-cli/bin/pangram.js", "utf8");
const expectedDependencies = {};

for (const { directory, os, cpu, libc } of releasePlatforms) {
  const manifest = JSON.parse(await readFile(`npm/${directory}/package.json`, "utf8"));
  if (manifest.version !== main.version) {
    throw new Error(`${manifest.name} version does not match the main package`);
  }
  if (JSON.stringify(manifest.os) !== JSON.stringify([os])) {
    throw new Error(`${manifest.name} has the wrong operating-system selector`);
  }
  if (JSON.stringify(manifest.cpu) !== JSON.stringify([cpu])) {
    throw new Error(`${manifest.name} has the wrong CPU selector`);
  }
  if (manifest.libc !== libc) {
    throw new Error(`${manifest.name} has the wrong libc selector`);
  }
  expectedDependencies[manifest.name] = main.version;
}

if (
  !wrapper.includes("`@microck/pangram-cli-${platform}`") ||
  !wrapper.includes("Object.hasOwn(manifest.optionalDependencies, packageName)")
) {
  throw new Error("platform wrapper does not derive and validate its package name");
}

const actualDependencies = Object.entries(main.optionalDependencies).sort();
const wantedDependencies = Object.entries(expectedDependencies).sort();
if (JSON.stringify(actualDependencies) !== JSON.stringify(wantedDependencies)) {
  throw new Error("main package optional dependencies do not match the platform packages");
}

for (const template of [
  "packaging/homebrew/pangram.rb.template",
  "packaging/scoop/pangram.json.template",
]) {
  const contents = await readFile(template, "utf8");
  if (!contents.includes("{{VERSION}}") || !contents.includes("Unofficial Pangram")) {
    throw new Error(`${template} is missing its version or unofficial-project disclosure`);
  }
}

for (const template of [
  "packaging/direct/pangram-installer.sh.template",
  "packaging/direct/pangram-installer.ps1.template",
]) {
  const contents = await readFile(template, "utf8");
  if (
    !contents.includes("{{VERSION}}") ||
    !contents.includes("__pangram-direct-install") ||
    !contents.includes("pangram-update-manifest.json.sig") ||
    !contents.includes("Unofficial Pangram") ||
    !contents.includes("releases/download/v") ||
    contents.includes("releases/latest/download")
  ) {
    throw new Error(`${template} is missing its release identity or verification handoff`);
  }
}

const releaseWorkflow = await readFile(".github/workflows/release.yml", "utf8");
const signedReplacementSmoke =
  "Run the signed direct-install and receipt-owned replacement smoke test";
if (releaseWorkflow.split(signedReplacementSmoke).length - 1 !== 2) {
  throw new Error("release workflow must prove signed replacement on Unix and Windows");
}
if (
  !releaseWorkflow.includes('! cmp -s "$root/initial-receipt.json" "$receipt"') ||
  !releaseWorkflow.includes("$replacementReceipt -ne $initialReceipt")
) {
  throw new Error("release workflow does not compare both replacement receipts");
}
if (
  !releaseWorkflow.includes(
    "sha256sum pangram-* pangram.rb THIRD-PARTY-LICENSES.txt >SHA256SUMS",
  )
) {
  throw new Error("release workflow does not checksum every public release asset");
}
if (
  !releaseWorkflow.includes('--target "$GITHUB_SHA"') ||
  !releaseWorkflow.includes('--notes-file "$RUNNER_TEMP/release-notes.md"')
) {
  throw new Error("release workflow does not pin its tag and generated notes input");
}
