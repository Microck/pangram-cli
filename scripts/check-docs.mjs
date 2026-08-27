import { access, readFile, readdir } from 'node:fs/promises';
import { dirname, extname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkDocumentationLinks } from './check-external-links.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const docsRoot = join(root, 'docs-app');
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

const docsFiles = await files(docsRoot);
const contentMarkdownFiles = docsFiles
  .filter((path) => path.startsWith(`${contentRoot}${sep}`))
  .filter((path) => ['.md', '.mdx'].includes(extname(path)));
const externalLinkFiles = [
  join(root, 'README.md'),
  ...(await files(join(root, 'docs'))).filter((path) => extname(path) === '.md'),
  ...contentMarkdownFiles,
];
failures.push(
  ...(await checkDocumentationLinks(externalLinkFiles, {
    displayRoot: root,
    sitePages: [{ path: '/', file: join(docsRoot, 'app/page.tsx') }],
    siteRoots: [
      { prefix: '/docs', root: contentRoot, sourceExtensions: ['.md', '.mdx'] },
      { prefix: '/', root: join(docsRoot, 'public') },
    ],
  })),
);
for (const path of docsFiles) {
  const text = await readFile(path, 'utf8');
  if (/[^\x00-\x7F]/.test(text)) failures.push(`${relative(root, path)} contains non-ASCII text`);
  if (/sk-pg-[A-Za-z0-9]+/.test(text)) failures.push(`${relative(root, path)} contains an API key`);
  for (const match of text.matchAll(/^pangram ([a-z][a-z-]*)(?:\s|$)/gm)) {
    if (!commands.has(match[1])) failures.push(`${relative(root, path)} names unknown command ${match[1]}`);
  }
}

for (const output of ['llms.txt', 'llms-full.txt']) {
  await access(join(root, 'docs-app/public', output)).catch(() => failures.push(`missing ${output}`));
}

const pagePaths = contentMarkdownFiles
  .map((path) => relative(contentRoot, path).replaceAll('\\', '/').replace(/\.mdx?$/, '.md'))
  .sort();
const markdownRoot = join(root, 'docs-app/public/markdown');
const markdownPaths = docsFiles
  .filter((path) => path.startsWith(`${markdownRoot}${sep}`))
  .filter((path) => extname(path) === '.md')
  .map((path) => relative(markdownRoot, path).replaceAll('\\', '/'))
  .sort();
const expectedMarkdown = new Set(pagePaths);
const actualMarkdown = new Set(markdownPaths);
for (const stale of markdownPaths.filter((path) => !expectedMarkdown.has(path))) {
  failures.push(`stale generated Markdown ${stale}`);
}
for (const missing of pagePaths.filter((path) => !actualMarkdown.has(path))) {
  failures.push(`missing generated Markdown ${missing}`);
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
