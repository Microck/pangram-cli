import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { releasePlatforms } from './release-platforms.mjs';
import { ensureUnpublishedPlatformEntries } from './version-finalizer.mjs';

test('version finalizer preserves exact unpublished platform lock entries', async () => {
  const root = await mkdtemp(join(tmpdir(), 'pangram-version-lock-'));
  try {
    await mkdir(join(root, 'npm/pangram-cli'), { recursive: true });
    const dependencies = {};
    for (const { directory } of releasePlatforms) {
      const name = `@microck/${directory}`;
      dependencies[name] = '1.3.2';
      await mkdir(join(root, 'npm', directory), { recursive: true });
      await writeFile(join(root, 'npm', directory, 'package.json'), JSON.stringify({name, version: '1.3.2'}));
    }
    await writeFile(join(root, 'npm/pangram-cli/package.json'), JSON.stringify({version:'1.3.2', optionalDependencies:dependencies}));
    const first = `npm/pangram-cli/node_modules/${Object.keys(dependencies)[0]}`;
    const lock = {lockfileVersion:3, packages:{[first]:{optional:true}}};
    await writeFile(join(root, 'package-lock.json'), JSON.stringify(lock));
    await ensureUnpublishedPlatformEntries(root);
    const result = JSON.parse(await readFile(join(root, 'package-lock.json'), 'utf8'));
    for (const name of Object.keys(dependencies)) {
      assert.deepEqual(result.packages[`npm/pangram-cli/node_modules/${name}`], {version:'1.3.2', optional:true});
    }
    result.packages[first].version = '1.3.1';
    await writeFile(join(root, 'package-lock.json'), JSON.stringify(result));
    await assert.rejects(ensureUnpublishedPlatformEntries(root), /unexpected version/);
  } finally { await rm(root, {recursive:true, force:true}); }
});
