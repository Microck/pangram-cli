import { mkdir, readFile, readdir, unlink, writeFile } from 'node:fs/promises';
import { dirname, extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const contentRoot = join(root, 'docs-app/content/docs');
const referenceRoot = join(contentRoot, 'reference');
const publicRoot = join(root, 'docs-app/public');
const markdownRoot = join(publicRoot, 'markdown');
const schemasRoot = join(publicRoot, 'schemas');
const cargoManifest = await readFile(join(root, 'Cargo.toml'), 'utf8');
const packageVersion = cargoManifest.match(/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/m)?.[1];
if (!packageVersion) throw new Error('Cargo.toml package version is missing or invalid');
const generatedNotice = `Generated from Pangram CLI ${packageVersion} and schema major 1.`;

const readJson = async (path) => JSON.parse(await readFile(join(root, path), 'utf8'));

/*
 * Generated output is committed, and this script is its only writer. An
 * unconditional write therefore destroys the on-disk copy before anyone can
 * diff it, and churns mtimes on every run even when nothing changed. Writing
 * only on a real byte difference keeps the run a true no-op when output is
 * already current, so the CI drift gate stays the only thing that flags drift.
 */
const writeIfChanged = async (path, content) => {
  const current = await readFile(path, 'utf8').catch(() => null);
  if (current === content) return;
  await writeFile(path, content);
};
const escapeCell = (value) => String(value).replaceAll('|', '\\|').replaceAll('\n', ' ');
const frontmatter = (title, description) => `---\ntitle: ${title}\ndescription: ${description}\n---\n\n`;

await mkdir(referenceRoot, { recursive: true });
await mkdir(schemasRoot, { recursive: true });

const cli = await readJson('generated/cli-reference.json');
const commandRows = cli.commands.map((command) => {
  const path = command.path.length === 0 ? 'pangram' : `pangram ${command.path.join(' ')}`;
  const argumentsText = command.arguments
    .map((argument) => {
      const value = argument.value_name ? ` ${argument.value_name}` : '';
      return `${argument.name}${value}`;
    })
    .join(', ');
  return `| \`${path}\` | ${command.kind} | ${command.availability} | ${escapeCell(argumentsText || 'none')} |`;
});
await writeIfChanged(
  join(referenceRoot, 'commands.mdx'),
  frontmatter('Command index', 'Generated command paths, availability, and arguments.') +
    `${generatedNotice}\n\n| Command | Kind | Availability | Arguments |\n| --- | --- | --- | --- |\n${commandRows.join('\n')}\n`,
);

const config = await readJson('contracts/config.schema.json');
const configRows = Object.entries(config.properties).map(([name, schema]) =>
  `| \`${name}\` | ${escapeCell(schema.type ?? schema.$ref ?? 'object')} | ${config.required?.includes(name) ? 'yes' : 'no'} |`,
);
await writeIfChanged(
  join(referenceRoot, 'configuration.mdx'),
  frontmatter('Configuration reference', 'Generated configuration schema fields and contract link.') +
    `${generatedNotice}\n\n| Field | Type | Required |\n| --- | --- | --- |\n${configRows.join('\n')}\n\n[Open the complete JSON Schema](/schemas/config.schema.json).\n`,
);

await writeIfChanged(
  join(referenceRoot, 'output-schema.mdx'),
  frontmatter('Output schema', 'Canonical success and failure envelope contract.') +
    `${generatedNotice}\n\nEvery structured CLI and MCP result starts from the canonical typed envelope.\n\n[Open the complete output JSON Schema](/schemas/output.schema.json).\n`,
);

const errors = await readJson('generated/error-reference.json');
const errorRows = errors.errors.map((error) =>
  `| \`${error.code}\` | ${error.category} | ${error.default_retryable ? 'yes' : 'no'} | ${error.contextual_retryability ? 'yes' : 'no'} |`,
);
await writeIfChanged(
  join(referenceRoot, 'errors.mdx'),
  frontmatter('Error catalog', 'Generated stable error codes, categories, and retry defaults.') +
    `${generatedNotice}\n\n| Code | Category | Retry by default | Contextual retry |\n| --- | --- | --- | --- |\n${errorRows.join('\n')}\n`,
);
await writeIfChanged(
  join(referenceRoot, 'exit-codes.mdx'),
  frontmatter('Exit codes', 'Generated stable process exit values.') +
    `${generatedNotice}\n\n${errors.exit_codes.map(({ code }) => `- \`${code}\``).join('\n')}\n`,
);

await writeIfChanged(
  join(referenceRoot, 'progress.mdx'),
  frontmatter('Progress events', 'Canonical stderr progress behavior for long operations.') +
    `${generatedNotice}\n\n\`--progress jsonl\` writes canonical progress envelopes to stderr. Final command output remains on stdout. Progress never includes credentials, submitted content, plagiarism matches, or public links.\n\n[Open the complete output JSON Schema](/schemas/output.schema.json).\n`,
);

const mcp = await readJson('generated/mcp-tools.json');
const toolRows = mcp.tools.map((tool) =>
  `| \`${tool.name}\` | ${escapeCell(tool.description)} | ${tool.annotations.readOnlyHint ? 'yes' : 'no'} | ${tool.annotations.openWorldHint ? 'yes' : 'no'} |`,
);
await writeIfChanged(
  join(referenceRoot, 'mcp-tools.mdx'),
  frontmatter('MCP tools', 'Generated MCP protocol version, tool inventory, and annotations.') +
    `${generatedNotice}\n\nProtocol: \`${mcp.protocol_version}\`.\n\n| Tool | Purpose | Read only | Open world |\n| --- | --- | --- | --- |\n${toolRows.join('\n')}\n\n[Open the generated tool contracts](/schemas/mcp-tools.json).\n`,
);

const updateManifest = await readJson('contracts/update-manifest.schema.json');
const updateRows = Object.entries(updateManifest.properties).map(([name, schema]) =>
  `| \`${name}\` | ${escapeCell(schema.type ?? schema.const ?? schema.$ref ?? 'value')} |`,
);
await writeIfChanged(
  join(referenceRoot, 'update-manifest.mdx'),
  frontmatter('Update manifest', 'Generated signed-manifest fields and verification boundary.') +
    `${generatedNotice}\n\nThe detached Ed25519 signature covers the exact downloaded manifest bytes.\n\n| Field | Contract |\n| --- | --- |\n${updateRows.join('\n')}\n\n[Open the complete update-manifest JSON Schema](/schemas/update-manifest.schema.json).\n`,
);

for (const [source, destination] of [
  ['contracts/config.schema.json', 'config.schema.json'],
  ['contracts/output.schema.json', 'output.schema.json'],
  ['contracts/update-manifest.schema.json', 'update-manifest.schema.json'],
  ['contracts/manifest-signature.schema.json', 'manifest-signature.schema.json'],
  ['contracts/update-state.schema.json', 'update-state.schema.json'],
  ['contracts/install-receipt.schema.json', 'install-receipt.schema.json'],
  ['generated/error-reference.json', 'error-reference.json'],
  ['generated/mcp-tools.json', 'mcp-tools.json'],
]) {
  await writeIfChanged(
    join(schemasRoot, destination),
    `${JSON.stringify(await readJson(source))}\n`,
  );
}

async function markdownFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await markdownFiles(path)));
    else if (['.md', '.mdx'].includes(extname(entry.name))) files.push(path);
  }
  return files.sort();
}

const pages = await markdownFiles(contentRoot);
const markdownPaths = new Map(
  pages.map((page) => {
    const pagePath = relative(contentRoot, page).replace(/\\/g, '/').replace(/\.mdx?$/, '');
    return [page, { pagePath, markdownPath: join(markdownRoot, `${pagePath}.md`) }];
  }),
);
const expectedMarkdown = new Set(
  [...markdownPaths.values()].map(({ markdownPath }) => markdownPath),
);
for (const generated of await markdownFiles(markdownRoot)) {
  if (!expectedMarkdown.has(generated)) await unlink(generated);
}
const navigation = [];
const full = [];
for (const page of pages) {
  const { pagePath, markdownPath } = markdownPaths.get(page);
  const text = await readFile(page, 'utf8');
  const body = text.replace(/^---\n[\s\S]*?\n---\n+/, '');
  const title = text.match(/^title: (.+)$/m)?.[1] ?? pagePath;
  navigation.push(`- [${title}](https://pangram.micr.dev/docs/${pagePath === 'index' ? '' : pagePath})`);
  full.push(`# ${title}\n\nSource: https://pangram.micr.dev/docs/${pagePath}\n\n${body.trim()}\n`);
  await mkdir(dirname(markdownPath), { recursive: true });
  await writeIfChanged(markdownPath, `# ${title}\n\n${body.trim()}\n`);
}

await writeIfChanged(
  join(publicRoot, 'llms.txt'),
  `# Pangram CLI\n\nUnofficial CLI, TUI, and stdio MCP server for documented Pangram APIs. Local history and public links are off by default. Billable MCP tools require an explicit ceiling.\n\n${navigation.join('\n')}\n`,
);
await writeIfChanged(join(publicRoot, 'llms-full.txt'), `${full.join('\n')}\n`);
