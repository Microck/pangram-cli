import { readFile, realpath, stat } from "node:fs/promises";
import { dirname, extname, join, normalize, relative, resolve, sep } from "node:path";
import { setTimeout as wait } from "node:timers/promises";
import { parseFragment } from "parse5";
import remarkGfm from "remark-gfm";
import remarkMdx from "remark-mdx";
import remarkParse from "remark-parse";
import { unified } from "unified";
import { SKIP, visit } from "unist-util-visit";

const excludedHosts = new Set(["pangram.micr.dev"]);
const retryableStatus = (status) => status === 429 || status >= 500;
const markdownParser = unified().use(remarkParse).use(remarkGfm);
const mdxParser = unified().use(remarkParse).use(remarkMdx).use(remarkGfm);
const ignoredNodeTypes = new Set([
  "code",
  "inlineCode",
  "mdxFlowExpression",
  "mdxTextExpression",
  "mdxjsEsm",
]);
const codeContainerNames = new Set(["code", "pre"]);
const linkAttributeNames = new Set(["href", "src"]);
const hasHttpScheme = (value) => /^https?:/i.test(value);
const isExternalUrl = (value) => /^https?:\/\//i.test(value);

function isWithin(root, target) {
  const absoluteRoot = resolve(root);
  const absoluteTarget = resolve(target);
  return absoluteTarget === absoluteRoot || absoluteTarget.startsWith(`${absoluteRoot}${sep}`);
}

function staticAttributeValue(attribute) {
  if (typeof attribute.value === "string") return attribute.value;
  const expression = attribute.value?.data?.estree?.body?.[0]?.expression;
  if (expression?.type === "Literal" && typeof expression.value === "string") {
    return expression.value;
  }
  if (expression?.type === "TemplateLiteral" && expression.expressions.length === 0) {
    return expression.quasis[0]?.value.cooked;
  }
  return undefined;
}

function appendLinkAttributes(destinations, attributes) {
  for (const attribute of attributes ?? []) {
    if (!linkAttributeNames.has(attribute.name)) continue;
    const value = staticAttributeValue(attribute);
    if (value !== undefined) destinations.push(value);
  }
}

function htmlDestinations(html) {
  const destinations = [];
  const visitHtml = (node) => {
    if (codeContainerNames.has(node.tagName)) return;
    appendLinkAttributes(destinations, node.attrs);
    for (const child of node.childNodes ?? []) visitHtml(child);
  };
  visitHtml(parseFragment(html));
  return destinations;
}

function codeContainerDelta(html) {
  const tag = html.trim();
  const match = /^<(\/)?(?:code|pre)\b[^>]*>$/i.exec(tag);
  if (!match || /\/\s*>$/.test(tag)) return 0;
  return match[1] ? -1 : 1;
}

function destinationsIn(markdown, isMdx) {
  const tree = (isMdx ? mdxParser : markdownParser).parse(markdown);
  const definitions = new Map();
  visit(tree, "definition", (node) => {
    if (!definitions.has(node.identifier)) definitions.set(node.identifier, node.url);
  });

  const links = [];
  let rawCodeDepth = 0;
  visit(tree, (node) => {
    if (ignoredNodeTypes.has(node.type)) return SKIP;
    if (
      ["mdxJsxFlowElement", "mdxJsxTextElement"].includes(node.type) &&
      codeContainerNames.has(node.name)
    ) {
      return SKIP;
    }
    if (node.type === "html") {
      const insideCode = rawCodeDepth > 0;
      rawCodeDepth = Math.max(0, rawCodeDepth + codeContainerDelta(node.value));
      if (!insideCode) links.push(...htmlDestinations(node.value));
      return undefined;
    }
    if (rawCodeDepth > 0) return SKIP;
    if (["image", "link"].includes(node.type)) links.push(node.url);
    if (["imageReference", "linkReference"].includes(node.type)) {
      const destination = definitions.get(node.identifier);
      if (destination !== undefined) links.push(destination);
    }
    if (["mdxJsxFlowElement", "mdxJsxTextElement"].includes(node.type)) {
      appendLinkAttributes(links, node.attributes);
    }
    return undefined;
  });
  return links;
}

function matchSiteRoot(pathname, siteRoots) {
  return siteRoots.find(
    ({ prefix }) => prefix === "/" || pathname === prefix || pathname.startsWith(`${prefix}/`),
  );
}

function candidatePaths(target, sitePage, siteRoot) {
  if (sitePage || (siteRoot && !siteRoot.sourceExtensions)) return [target];
  if (siteRoot) {
    return [
      ...siteRoot.sourceExtensions.map((extension) => `${target}${extension}`),
      ...siteRoot.sourceExtensions.map((extension) => join(target, `index${extension}`)),
    ];
  }
  return [
    target,
    `${target}.md`,
    `${target}.mdx`,
    join(target, "index.md"),
    join(target, "index.mdx"),
  ];
}

async function localLinkFailure(source, destination, displayRoot, siteRoots, sitePages) {
  if (
    !destination ||
    destination.startsWith("#") ||
    destination.startsWith("?") ||
    /^[a-z][a-z+.-]*:/i.test(destination)
  ) {
    return undefined;
  }
  let pathname;
  try {
    pathname = decodeURIComponent(destination.split(/[?#]/, 1)[0]);
  } catch {
    return `${relative(displayRoot, source)} has invalid relative link ${destination}`;
  }
  const isSitePath = destination.startsWith("/");
  const sitePage = isSitePath ? sitePages.get(pathname) : undefined;
  const absoluteSiteRoot =
    destination.startsWith("/") && !sitePage ? matchSiteRoot(pathname, siteRoots) : undefined;
  const relativeSiteRoot = isSitePath
    ? undefined
    : siteRoots.find(
        ({ root, sourceExtensions }) => sourceExtensions && isWithin(root, source),
      );
  const siteRoot = absoluteSiteRoot ?? relativeSiteRoot;
  if (isSitePath && !sitePage && !siteRoot) {
    return `${relative(displayRoot, source)} has unresolved link ${destination}`;
  }
  const relativePath = absoluteSiteRoot
    ? pathname
        .slice(absoluteSiteRoot.prefix === "/" ? 1 : absoluteSiteRoot.prefix.length)
        .replace(/^\/+|\/+$/g, "")
    : pathname;
  if (
    siteRoot?.sourceExtensions &&
    (relativePath.split("/").at(-1) === "index" ||
      siteRoot.sourceExtensions.some((extension) =>
        relativePath.toLowerCase().endsWith(extension.toLowerCase()),
      ))
  ) {
    return `${relative(displayRoot, source)} has unresolved link ${destination}`;
  }
  const applicableRoot = siteRoot?.root ?? displayRoot;
  const target = normalize(
    sitePage ??
      (absoluteSiteRoot
        ? join(applicableRoot, relativePath)
        : join(dirname(source), relativePath)),
  );
  if (!isWithin(applicableRoot, target)) {
    return `${relative(displayRoot, source)} has unresolved link ${destination}`;
  }
  const candidates = candidatePaths(target, sitePage, siteRoot);
  const canonicalRoot = await realpath(applicableRoot);
  const resolved = await Promise.all(
    candidates.map(async (candidate) => {
      try {
        const canonicalCandidate = await realpath(candidate);
        if ((sitePage || siteRoot) && !(await stat(canonicalCandidate)).isFile()) return false;
        return isWithin(canonicalRoot, canonicalCandidate);
      } catch {
        return false;
      }
    }),
  ).then((results) => results.some(Boolean));
  return resolved ? undefined : `${relative(displayRoot, source)} has unresolved link ${destination}`;
}

async function probe(url) {
  let lastFailure;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      const response = await fetch(url, {
        headers: {
          accept: "text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.1",
          "user-agent": "pangram-cli-link-check/0.1",
        },
        redirect: "follow",
        signal: AbortSignal.timeout(20_000),
      });
      await response.body?.cancel().catch(() => {});
      if (response.status >= 200 && response.status < 400) return undefined;
      lastFailure = `returned HTTP ${response.status}`;
      if (!retryableStatus(response.status)) return lastFailure;
    } catch (error) {
      lastFailure = `failed: ${error.message}`;
    }
    if (attempt < 2) await wait(500 * (attempt + 1));
  }
  return lastFailure;
}

/** Validates repository-relative destinations and probes each unique external URL once. */
export async function checkDocumentationLinks(
  paths,
  { displayRoot = process.cwd(), sitePages = [], siteRoots = [] } = {},
) {
  const sourcesByUrl = new Map();
  const failures = [];
  const orderedSiteRoots = [...siteRoots].sort(
    (left, right) => right.prefix.length - left.prefix.length,
  );
  const sitePageFiles = new Map(sitePages.map(({ path, file }) => [path, file]));
  for (const path of paths) {
    const markdown = await readFile(path, "utf8");
    for (const value of new Set(destinationsIn(markdown, extname(path) === ".mdx"))) {
      if (hasHttpScheme(value) && !isExternalUrl(value)) {
        failures.push(`${relative(displayRoot, path)}: invalid external URL ${value}`);
        continue;
      }
      if (!isExternalUrl(value)) {
        const failure = await localLinkFailure(
          path,
          value,
          displayRoot,
          orderedSiteRoots,
          sitePageFiles,
        );
        if (failure) failures.push(failure);
        continue;
      }
      const url = URL.parse(value);
      if (!url) {
        failures.push(`${relative(displayRoot, path)}: invalid external URL ${value}`);
        continue;
      }
      if (excludedHosts.has(url.hostname)) continue;
      const sources = sourcesByUrl.get(url.href) ?? new Set();
      sources.add(relative(displayRoot, path));
      sourcesByUrl.set(url.href, sources);
    }
  }

  const entries = [...sourcesByUrl.entries()];
  let next = 0;
  const workers = Array.from({ length: Math.min(6, entries.length) }, async () => {
    while (next < entries.length) {
      const [url, sources] = entries[next];
      next += 1;
      const failure = await probe(url);
      if (failure) {
        failures.push(`${[...sources].sort().join(", ")}: ${url} ${failure}`);
      }
    }
  });
  await Promise.all(workers);
  return failures.sort();
}
