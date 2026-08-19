import { access, readFile, readdir } from 'node:fs/promises';
import { dirname, extname, join, normalize, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const contentRoot = join(root, 'docs-app/content/docs');
const cli = JSON.parse(await readFile(join(root, 'generated/cli-reference.json'), 'utf8'));
const commands = new Set(cli.commands.map(({ path }) => path[0]).filter(Boolean));
const failures = [];

async function files(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const found = [];
  for (const entry of entries) {
    if (entry.isDirectory() && ['.next', 'node_modules'].includes(entry.name)) continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...(await files(path)));
    else if (['.md', '.mdx', '.ts', '.tsx', '.js', '.mjs', '.json', '.css'].includes(extname(path))) found.push(path);
  }
  return found;
}

for (const path of await files(join(root, 'docs-app'))) {
  const text = await readFile(path, 'utf8');
  if (/[^\x00-\x7F]/.test(text)) failures.push(`${relative(root, path)} contains non-ASCII text`);
  if (/sk-pg-[A-Za-z0-9]+/.test(text)) failures.push(`${relative(root, path)} contains an API key`);
  for (const match of text.matchAll(/^pangram ([a-z][a-z-]*)(?:\s|$)/gm)) {
    if (!commands.has(match[1])) failures.push(`${relative(root, path)} names unknown command ${match[1]}`);
  }
  for (const match of text.matchAll(/\]\((\.\.?\/[^)#]+)(?:#[^)]+)?\)/g)) {
    const target = normalize(join(dirname(path), match[1]));
    const candidates = [target, `${target}.mdx`, `${target}.md`, join(target, 'index.mdx')];
    if (!(await Promise.any(candidates.map((candidate) => access(candidate))).then(() => true, () => false))) {
      failures.push(`${relative(root, path)} has unresolved link ${match[1]}`);
    }
  }
}

for (const output of ['llms.txt', 'llms-full.txt']) {
  await access(join(root, 'docs-app/public', output)).catch(() => failures.push(`missing ${output}`));
}

for (const route of ['app/page.tsx', 'app/docs/layout.tsx', 'app/docs/[[...slug]]/page.tsx']) {
  await access(join(root, 'docs-app', route)).catch(() => failures.push(`missing route ${route}`));
}
const sourceConfig = await readFile(join(root, 'docs-app/lib/source.ts'), 'utf8');
if (!sourceConfig.includes("baseUrl: '/docs'")) {
  failures.push('Fumadocs source base URL must remain /docs');
}

if (failures.length > 0) {
  throw new Error(`Documentation checks failed:\n${failures.join('\n')}`);
}
