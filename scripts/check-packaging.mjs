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

const ciWorkflow = await readFile(".github/workflows/ci.yml", "utf8");
const releaseBuildAction = await readFile(
  ".github/actions/build-release-archive/action.yml",
  "utf8",
);
const releaseWorkflow = await readFile(".github/workflows/release.yml", "utf8");
const npmPackScript = await readFile("scripts/pack-npm-release.mjs", "utf8");
const homebrewSmoke = await readFile("scripts/smoke-homebrew-release.sh", "utf8");
const scoopSmoke = await readFile("scripts/smoke-scoop-release.ps1", "utf8");
const signedReplacementSmoke =
  "Run the public signed installer and receipt-owned replacement smoke test";
if (releaseWorkflow.split(signedReplacementSmoke).length - 1 !== 2) {
  throw new Error("release workflow must prove the public installers on Unix and Windows");
}
if (
  !releaseWorkflow.includes("sh release-artifacts/pangram-installer.sh") ||
  !releaseWorkflow.includes('& "release-artifacts/pangram-installer.ps1"') ||
  releaseWorkflow.includes("__pangram-direct-install")
) {
  throw new Error("release workflow bypasses a generated public installer");
}
if (
  !ciWorkflow.includes("uses: ./.github/actions/build-release-archive") ||
  !releaseWorkflow.includes("uses: ./.github/actions/build-release-archive")
) {
  throw new Error("CI and release workflows must use the canonical release build action");
}
for (const requiredBuildProof of [
  'cargo zigbuild --locked --release --target "${TARGET}.2.17" --bin pangram',
  'test "$(getconf GNU_LIBC_VERSION)" = "glibc 2.17"',
]) {
  if (!releaseBuildAction.includes(requiredBuildProof)) {
    throw new Error(`release build action is missing proof: ${requiredBuildProof}`);
  }
}
if (
  !npmPackScript.includes('from "./release-platforms.mjs"') ||
  !npmPackScript.includes('resolve(destination, "packages.json")') ||
  !releaseWorkflow.includes(
    'node scripts/pack-npm-release.mjs release-verification/npm "$VERSION"',
  )
) {
  throw new Error("release workflow does not pack npm targets from the canonical platform map");
}
for (const requiredChannelProof of [
  "npm install --ignore-scripts --no-audit --no-fund --offline --omit=optional",
  'bash release-artifacts/smoke-homebrew-release.sh release-artifacts "$VERSION"',
  '& "release-artifacts/smoke-scoop-release.ps1"',
]) {
  if (!releaseWorkflow.includes(requiredChannelProof)) {
    throw new Error(`release workflow is missing channel proof: ${requiredChannelProof}`);
  }
}
for (const [script, contents, requiredProofs] of [
  [
    "scripts/smoke-homebrew-release.sh",
    homebrewSmoke,
    [
      'brew tap-new --no-git "$tap"',
      'brew install "$tap/pangram"',
      'brew test "$tap/pangram"',
      '"pangram ${version}"',
    ],
  ],
  [
    "scripts/smoke-scoop-release.ps1",
    scoopSmoke,
    [
      "& $scoop install --no-update-scoop $manifest",
      '(Join-Path $env:SCOOP "shims/pangram.exe") --version',
      '$installed -ne "pangram $Version"',
      "& $scoop uninstall pangram",
    ],
  ],
]) {
  for (const requiredProof of requiredProofs) {
    if (!contents.includes(requiredProof)) {
      throw new Error(`${script} is missing channel proof: ${requiredProof}`);
    }
  }
}
if (homebrewSmoke.includes("HOMEBREW_NO_INSTALL_FROM_API")) {
  throw new Error("Homebrew smoke must allow API-backed core metadata");
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
