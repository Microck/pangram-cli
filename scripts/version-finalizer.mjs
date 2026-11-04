import { execFile } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { releasePlatforms } from './release-platforms.mjs';

const run = promisify(execFile);

// npm cannot resolve the next version's unpublished optional packages yet.
// Keep exact-version placeholders so npm ci can validate the workspace before
// publication; npm resolves the scoped package from the registry once present.
export async function ensureUnpublishedPlatformEntries(root = process.cwd()) {
  const lockPath = join(root, 'package-lock.json');
  const lock = JSON.parse(await readFile(lockPath, 'utf8'));
  const wrapper = JSON.parse(await readFile(join(root, 'npm/pangram-cli/package.json'), 'utf8'));
  for (const { directory } of releasePlatforms) {
    const manifest = JSON.parse(await readFile(join(root, 'npm', directory, 'package.json'), 'utf8'));
    if (manifest.version !== wrapper.version || wrapper.optionalDependencies[manifest.name] !== wrapper.version) {
      throw new Error(`Platform package ${manifest.name} is not synchronized with the wrapper`);
    }
    const key = `npm/pangram-cli/node_modules/${manifest.name}`;
    const existing = lock.packages[key] ?? lock.packages[`node_modules/${manifest.name}`];
    if (existing?.version && existing.version !== wrapper.version) {
      throw new Error(`Lock entry for ${manifest.name} has unexpected version ${existing.version}`);
    }
    if (!existing?.version) lock.packages[key] = { version: wrapper.version, optional: true };
  }
  await writeFile(lockPath, `${JSON.stringify(lock, null, 2)}\n`);
}

export function versionFinalizerPlugin(root = process.cwd()) {
  return {
    name: 'pangram-version-finalizer',
    async applyCliDraft() {
      // This plugin is ordered after npm's lock update and before GitHub's
      // version-request plugin snapshots the branch.
      await ensureUnpublishedPlatformEntries(root);
      await run(process.execPath, ['scripts/generate-docs.mjs'], { cwd: root });
    },
  };
}
