import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";

import { checkDocumentationLinks } from "./check-external-links.mjs";

let server;
let origin;
let fixtureRoot;

before(async () => {
  fixtureRoot = await mkdtemp(join(tmpdir(), "pangram-doc-links-"));
  server = createServer((request, response) => {
    const works = ["/ok", "/wiki/Foo_(bar)"].includes(request.url);
    response.writeHead(works ? 204 : 404, { connection: "close" }).end();
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  origin = `http://127.0.0.1:${address.port}`;
});

after(async () => {
  await new Promise((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
  await rm(fixtureRoot, { recursive: true, force: true });
});

test("external link checks reject broken prose links only", async () => {
  const fixture = join(fixtureRoot, "links.md");
  await writeFile(join(fixtureRoot, "local.md"), "# Local\n");
  await writeFile(
    fixture,
    [
      `[working](${origin}/ok)`,
      `${origin}/wiki/Foo_(bar)`,
      `[working reference][ok-reference]`,
      `[relative](./local.md)`,
      `[relative reference][local-reference]`,
      `\`${origin}/ignored-inline\``,
      `\`\`${origin}/ignored-multi-backtick\`inside\`\``,
      `<code>${origin}/ignored-html-code</code>`,
      `<pre><a href="${origin}/ignored-html-pre">ignored</a></pre>`,
      "````bash",
      "```",
      `${origin}/ignored-fence`,
      "````",
      "https://pangram.micr.dev/docs",
      "",
      `[ok-reference]: ${origin}/ok`,
      `[ok-reference]: ${origin}/ignored-shadowed-definition`,
      `[local-reference]: ./local.md`,
      `[unused-reference]: ${origin}/ignored-unused-definition`,
    ].join("\n"),
  );
  const options = { displayRoot: fixtureRoot };
  assert.deepEqual(await checkDocumentationLinks([fixture], options), []);

  await writeFile(fixture, `[broken](${origin}/missing)\n`);
  const failures = await checkDocumentationLinks([fixture], options);
  assert.equal(failures.length, 1);
  assert.match(failures[0], /\/missing returned HTTP 404/);

  await writeFile(fixture, "[invalid](https://[)\n");
  assert.match((await checkDocumentationLinks([fixture], options))[0], /invalid external URL/);
});

test("documentation link checks cover repository-relative, HTML, and MDX links", async () => {
  const markdown = join(fixtureRoot, "prose.md");
  const mdx = join(fixtureRoot, "prose.mdx");

  await writeFile(
    markdown,
    `[missing](missing.md)\n<a href="${origin}/missing">broken</a>\n`,
  );
  await writeFile(
    mdx,
    [
      `<Card href="${origin}/missing">broken</Card>`,
      `<Card href={"${origin}/missing-expression"}>broken expression</Card>`,
      `<code>${origin}/ignored-mdx-code</code>`,
      `<pre><a href="${origin}/ignored-mdx-pre">ignored</a></pre>`,
    ].join("\n"),
  );

  const failures = await checkDocumentationLinks([markdown, mdx], { displayRoot: fixtureRoot });
  assert.equal(failures.length, 3);
  assert.ok(failures.some((failure) => failure.includes("unresolved link missing.md")));
  assert.ok(failures.some((failure) => failure.includes("/missing returned HTTP 404")));
  assert.ok(failures.some((failure) => failure.includes("/missing-expression returned HTTP 404")));
});

test("documentation link checks reject broken site-root and malformed HTTP links", async () => {
  const fixture = join(fixtureRoot, "site-root.md");
  const publicRoot = join(fixtureRoot, "public");
  await mkdir(join(publicRoot, "schemas"), { recursive: true });
  await writeFile(join(publicRoot, "schemas", "valid.json"), "{}\n");
  await writeFile(join(publicRoot, "guide.md"), "# Guide\n");
  await writeFile(
    fixture,
    [
      "[valid](/schemas/valid.json)",
      "[missing public extension](/guide)",
      "[missing](/schemas/missing.json)",
      "[malformed](https:/example.com/guide)",
    ].join("\n"),
  );

  const failures = await checkDocumentationLinks([fixture], {
    displayRoot: fixtureRoot,
    siteRoots: [{ prefix: "/", root: publicRoot }],
  });
  assert.equal(failures.length, 3);
  assert.ok(failures.some((failure) => failure.includes("unresolved link /guide")));
  assert.ok(failures.some((failure) => failure.includes("unresolved link /schemas/missing.json")));
  assert.ok(failures.some((failure) => failure.includes("invalid external URL https:/example.com/guide")));
});

test("documentation link checks reject paths outside their declared roots", async () => {
  const repositoryRoot = join(fixtureRoot, "repository");
  const publicRoot = join(repositoryRoot, "public");
  const source = join(repositoryRoot, "docs", "escape.md");
  await mkdir(join(repositoryRoot, "docs"), { recursive: true });
  await mkdir(publicRoot, { recursive: true });
  await writeFile(join(fixtureRoot, "outside.md"), "# Outside\n");
  await writeFile(join(repositoryRoot, "outside-public.json"), "{}\n");
  await symlink(join(fixtureRoot, "outside.md"), join(repositoryRoot, "docs", "outside-link.md"));
  await symlink(
    join(repositoryRoot, "outside-public.json"),
    join(publicRoot, "outside-link.json"),
  );
  await writeFile(
    source,
    [
      "[repository escape](../../outside.md)",
      "[public escape](/../outside-public.json)",
      "[repository symlink](outside-link.md)",
      "[public symlink](/outside-link.json)",
    ].join("\n"),
  );

  const failures = await checkDocumentationLinks([source], {
    displayRoot: repositoryRoot,
    siteRoots: [{ prefix: "/", root: publicRoot }],
  });
  assert.equal(failures.length, 4);
  assert.ok(failures.every((failure) => failure.includes("unresolved link")));
});

test("documentation link checks resolve Fumadocs routes against content", async () => {
  const repositoryRoot = join(fixtureRoot, "fumadocs-repository");
  const contentRoot = join(repositoryRoot, "content");
  const publicRoot = join(repositoryRoot, "public");
  const landingPage = join(repositoryRoot, "app", "page.tsx");
  const source = join(contentRoot, "index.mdx");
  const outputSource = join(contentRoot, "reference", "output-schema.mdx");
  const nestedSource = join(contentRoot, "leaf", "page.mdx");
  await mkdir(join(contentRoot, "reference"), { recursive: true });
  await mkdir(join(contentRoot, "leaf"), { recursive: true });
  await mkdir(join(contentRoot, "empty"), { recursive: true });
  await mkdir(publicRoot, { recursive: true });
  await mkdir(join(repositoryRoot, "app"), { recursive: true });
  await writeFile(landingPage, "export default function Page() {}\n");
  await writeFile(join(contentRoot, "reference", "index.mdx"), "# Reference\n");
  await writeFile(outputSource, "# Output schema\n");
  await writeFile(nestedSource, "[valid query-only link](?view=json)\n");
  await writeFile(
    source,
    [
      "[valid](/docs/reference/output-schema)",
      "[valid trailing slash](/docs/reference/output-schema/)",
      "[source extension](/docs/reference/output-schema.mdx)",
      "[root index slug](/docs/index)",
      "[nested index slug](/docs/reference/index)",
      "[landing page](/)",
      "[valid relative directory page](reference)",
      "[missing relative directory page](empty)",
      "[valid directory page](/docs/reference)",
      "[missing](/docs/reference/outpt-schema)",
    ].join("\n"),
  );

  const failures = await checkDocumentationLinks([source, nestedSource], {
    displayRoot: repositoryRoot,
    siteRoots: [
      { prefix: "/docs", root: contentRoot, sourceExtensions: [".md", ".mdx"] },
      { prefix: "/", root: publicRoot },
    ],
    sitePages: [{ path: "/", file: landingPage }],
  });
  assert.equal(failures.length, 5);
  assert.ok(
    failures.some((failure) =>
      failure.endsWith("unresolved link /docs/reference/output-schema.mdx"),
    ),
  );
  assert.ok(failures.some((failure) => failure.endsWith("unresolved link /docs/index")));
  assert.ok(
    failures.some((failure) => failure.endsWith("unresolved link /docs/reference/index")),
  );
  assert.ok(failures.some((failure) => failure.endsWith("unresolved link empty")));
  assert.ok(
    failures.some((failure) => failure.endsWith("unresolved link /docs/reference/outpt-schema")),
  );
});
