import { chmod, readFile, writeFile } from "node:fs/promises";
import { basename, resolve } from "node:path";
import { releasePlatforms } from "./release-platforms.mjs";

const artifacts = resolve(process.argv[2] ?? "release-artifacts");
const version = process.argv[3];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  throw new Error("usage: node scripts/render-package-manifests.mjs ARTIFACTS VERSION");
}

const signedManifest = JSON.parse(
  await readFile(resolve(artifacts, "pangram-update-manifest.json"), "utf8"),
);
if (signedManifest.version !== version || signedManifest.artifacts?.length !== 5) {
  throw new Error("signed manifest does not match the requested release");
}

const metadataByTarget = new Map();
for (const { target } of releasePlatforms) {
  const metadata = JSON.parse(
    await readFile(resolve(artifacts, `${target}.artifact.json`), "utf8"),
  );
  if (metadata.target !== target) throw new Error(`metadata target mismatch for ${target}`);
  if (!Number.isSafeInteger(metadata.size_bytes) || metadata.size_bytes < 1) {
    throw new Error(`metadata size mismatch for ${target}`);
  }
  const signed = signedManifest.artifacts.find((artifact) => artifact.target === target);
  if (
    !signed ||
    signed.sha256 !== metadata.sha256 ||
    signed.size_bytes !== metadata.size_bytes ||
    basename(new URL(signed.url).pathname) !== metadata.file_name
  ) {
    throw new Error(`signed artifact mismatch for ${target}`);
  }
  metadataByTarget.set(target, {
    file_name: metadata.file_name,
    sha256: signed.sha256,
    size_bytes: signed.size_bytes,
  });
}

async function render(source, destination, executable = false) {
  let template = await readFile(resolve(source), "utf8");
  template = template.replaceAll("{{VERSION}}", version);
  for (const [target, metadata] of metadataByTarget) {
    const suffix = target.replaceAll("-", "_").toUpperCase();
    template = template
      .replaceAll(`{{SHA256_${suffix}}}`, metadata.sha256)
      .replaceAll(`{{SIZE_${suffix}}}`, String(metadata.size_bytes))
      .replaceAll(`{{FILE_${suffix}}}`, metadata.file_name);
  }
  if (template.includes("{{")) throw new Error(`unresolved placeholder in ${source}`);
  await writeFile(resolve(artifacts, destination), template);
  if (executable) await chmod(resolve(artifacts, destination), 0o755);
}

await render("packaging/homebrew/pangram.rb.template", "pangram.rb");
await render("packaging/scoop/pangram.json.template", "pangram-scoop.json");
await render(
  "packaging/direct/pangram-installer.sh.template",
  "pangram-installer.sh",
  true,
);
await render(
  "packaging/direct/pangram-installer.ps1.template",
  "pangram-installer.ps1",
);
