import type { Metadata } from 'next';
import { IBM_Plex_Mono, IBM_Plex_Sans, Imbue } from 'next/font/google';
import Link from 'next/link';

import './landing.css';

/*
 * The landing route follows the Pangram design system (Imbue display serif,
 * IBM Plex for body and code, warm off-white surface, one orange action). The
 * docs surface keeps its own dark theme, so the fonts and tokens are attached
 * to this page's root element rather than to the shared root layout.
 */
const sans = IBM_Plex_Sans({
  subsets: ['latin'],
  weight: ['400', '500'],
  variable: '--pg-font-sans',
  display: 'swap',
});

const mono = IBM_Plex_Mono({
  subsets: ['latin'],
  weight: ['400', '500'],
  variable: '--pg-font-mono',
  display: 'swap',
});

const display = Imbue({
  subsets: ['latin'],
  variable: '--pg-font-display',
  display: 'swap',
});

export const metadata: Metadata = {
  title: { absolute: 'Pangram CLI - AI detection from the terminal' },
  description:
    'Unofficial terminal client for Pangram AI detection and plagiarism checking. One Rust core serves shell pipelines, a TUI, and a local stdio MCP server.',
};

const surfaces = [
  {
    title: 'Shell pipelines',
    meta: 'printf ... | pangram',
    body: 'Piped stdin runs AI detection and prints the canonical JSON envelope. Repeated files stream as JSONL so every result stays independently parseable.',
  },
  {
    title: 'Terminal UI',
    meta: 'pangram',
    body: 'An all-TTY launch opens the interactive interface: paste or load a document, watch progress, read segment evidence. Local history is opt-in.',
  },
  {
    title: 'Local MCP server',
    meta: 'pangram mcp',
    body: 'A typed stdio server on protocol 2026-07-28. Tools map to analysis operations instead of shelling out to the CLI.',
  },
];

const detectionFields = [
  { key: 'classification', value: 'ai, human, or mixed' },
  {
    key: 'fraction_ai, fraction_ai_assisted, fraction_human',
    value: 'Document composition, each 0.0 through 1.0',
  },
  {
    key: 'num_ai_segments, num_ai_assisted_segments, num_human_segments',
    value: 'Segment counts behind the fractions',
  },
  {
    key: 'segments[]',
    value: 'Per-segment label, confidence, character indices, word and token counts, humanizer score',
  },
  { key: 'headline, prediction', value: "Pangram's own summary of the document" },
  { key: 'dashboard_link', value: 'Present only when a public link is requested for that submission' },
];

const plagiarismFields = [
  { key: 'plagiarism_detected', value: 'Whether any sentence matched a source' },
  {
    key: 'total_sentences, plagiarized_sentence_count, percent_plagiarized',
    value: 'Coverage of the checked document',
  },
  { key: 'matches[]', value: 'Matched sentences with their online sources' },
];

const commands = [
  {
    name: 'pangram',
    purpose: 'Literal text or piped stdin runs detection. An all-TTY launch opens the TUI.',
  },
  { name: 'pangram detect', purpose: 'Pangram 4 AI text detection, including PDF, DOCX, and RTF input.' },
  { name: 'pangram plagiarism', purpose: 'Plagiarism checking on text.' },
  { name: 'pangram analyze', purpose: 'AI detection and plagiarism together, on the same text.' },
  { name: 'pangram bulk', purpose: 'Asynchronous bulk detection: submit, status, wait, results.' },
  { name: 'pangram history', purpose: 'List, show, search, delete, clear, export, and rerun local history.' },
  { name: 'pangram mcp', purpose: 'Run or install the typed stdio MCP server.' },
  { name: 'pangram doctor', purpose: 'Local, non-billable diagnostics.' },
];

const prettyOutput = `Pangram
  schema_version: 1
  command: detect

Analysis
  id: anl_01983c20-0180-7a80-a001-000000000001
  status: succeeded
  input: file essay.md 812 words

Checks
  kind: ai_detection
  status: succeeded
  headline: Likely AI-generated
  classification: ai
  fraction_ai: 0.97
  fraction_human: 0.03
  confidence: high`;

const errorEnvelope = `{
  "schema_version": "1",
  "command": "detect",
  "error": {
    "code": "missing_api_key",
    "category": "authentication",
    "message": "No Pangram API key is configured.",
    "retryable": false,
    "recovery": {
      "message": "Configure a persistent key or set PANGRAM_API_KEY.",
      "command": "pangram auth"
    }
  },
  "meta": { "failed_at": "2026-07-23T12:00:00Z" }
}`;

export default function Home() {
  return (
    <div data-pangram-landing className={`${sans.variable} ${mono.variable} ${display.variable}`}>
      <header className="pg-shell">
        <nav className="pg-nav">
          <Link href="/" className="pg-wordmark">
            pangram cli
            <span className="pg-badge">Unofficial</span>
          </Link>
          <div className="pg-nav__links">
            <Link href="/docs">Docs</Link>
            <Link href="/docs/reference/commands">Commands</Link>
            <Link href="/docs/reference/mcp-tools">MCP</Link>
            <a href="https://github.com/Microck/pangram-cli">GitHub</a>
          </div>
        </nav>
      </header>

      <main>
        <section className="pg-shell pg-hero">
          <div className="pg-hero__copy">
            <p className="pg-eyebrow">Unofficial client for Pangram AI detection</p>
            <h1 className="pg-display">AI detection and plagiarism, from the terminal.</h1>
            <p className="pg-lede">
              One Rust core serves shell pipelines, an interactive TUI, and a local stdio MCP server. JSON is the
              default. Schemas, error codes, and exit codes are contracts, not decoration.
            </p>
            <div className="pg-actions">
              <Link href="/docs/tutorials/first-tui-analysis" className="pg-button pg-button--primary">
                Read the docs
              </Link>
              <Link href="/docs/reference/commands" className="pg-button pg-button--secondary">
                Command reference
              </Link>
            </div>
            <p className="pg-small">
              No public package or installer is released yet. Public distribution waits on Pangram's written
              permission.{' '}
              <Link href="/docs/how-to/install" className="pg-link">
                Build from source
              </Link>
              .
            </p>
          </div>

          <div className="pg-demo">
            <div className="pg-demo__bar">
              <span className="pg-demo__dot" />
              <span className="pg-demo__dot" />
              <span className="pg-demo__dot" />
              <span className="pg-demo__title">pangram detect --format pretty --file essay.md</span>
            </div>
            <div className="pg-demo__body">
              <pre>{prettyOutput}</pre>
            </div>
            <div className="pg-demo__verdict">
              <span className="pg-state pg-state--ai" />
              <span>classification: ai</span>
            </div>
            <p className="pg-demo__caption">
              JSON is the default outside the TUI. Pretty, Markdown, TOON, and JSONL are opt-in projections of the
              same envelope.
            </p>
          </div>
        </section>

        <section className="pg-shell pg-section">
          <p className="pg-eyebrow">One core, three surfaces</p>
          <h2 className="pg-heading">Same analysis module behind every entry point.</h2>
          <p className="pg-body pg-lede">
            A single deep Rust module owns Pangram HTTP behavior, polling, normalization, retries, and task state. The
            CLI, TUI, and MCP server are adapters over it, so a result does not change shape because of how you asked
            for it.
          </p>
          <div className="pg-cards">
            {surfaces.map((surface) => (
              <div key={surface.title} className="pg-card">
                <h3 className="pg-subheading">{surface.title}</h3>
                <p className="pg-card__meta">{surface.meta}</p>
                <p className="pg-small">{surface.body}</p>
              </div>
            ))}
          </div>
          <img
            className="pg-band"
            src="/illustrations/one-core-platforms-1800.webp"
            srcSet="/illustrations/one-core-platforms-900.webp 900w, /illustrations/one-core-platforms-1800.webp 1800w"
            sizes="(min-width: 1264px) 1200px, 100vw"
            width={1800}
            height={1008}
            alt="Illustration of differently shaped trains arriving at one shared station platform."
            loading="lazy"
            decoding="async"
          />
        </section>

        <section className="pg-shell pg-section">
          <div className="pg-split">
            <div>
              <p className="pg-eyebrow">Evidence, not verdicts</p>
              <h2 className="pg-heading">Segment-level results you can audit.</h2>
              <p className="pg-lede">
                Detection returns document composition and per-segment evidence, not a single number. Results are
                probabilistic and belong alongside other signals, never on their own.
              </p>
              <p className="pg-links">
                <Link href="/docs/explanation/evidence" className="pg-link">
                  How to read the evidence
                </Link>
                <Link href="/docs/reference/output-schema" className="pg-link">
                  Output schema
                </Link>
              </p>
            </div>
            <img
              className="pg-band"
              src="/illustrations/segment-evidence-1800.webp"
              srcSet="/illustrations/segment-evidence-900.webp 900w, /illustrations/segment-evidence-1800.webp 1800w"
              sizes="(min-width: 1024px) 700px, 100vw"
              width={1800}
              height={1008}
              alt="Illustration of a greenhouse divided into equal glass sections, each holding different plants."
              loading="lazy"
              decoding="async"
            />
          </div>
          <div className="pg-fields">
            {detectionFields.map((field) => (
              <div key={field.key} className="pg-fields__row">
                <span className="pg-fields__key">{field.key}</span>
                <span className="pg-fields__value">{field.value}</span>
              </div>
            ))}
          </div>
          <p className="pg-eyebrow">Plagiarism check</p>
          <div className="pg-fields">
            {plagiarismFields.map((field) => (
              <div key={field.key} className="pg-fields__row">
                <span className="pg-fields__key">{field.key}</span>
                <span className="pg-fields__value">{field.value}</span>
              </div>
            ))}
          </div>
        </section>

        <section className="pg-shell pg-section">
          <p className="pg-eyebrow">Built for agents</p>
          <h2 className="pg-heading">Typed MCP tools with the dangerous parts gated.</h2>
          <div className="pg-split">
            <div>
              <p className="pg-lede">
                Seventeen typed tools on protocol 2026-07-28. Billable tools are marked non-idempotent and refuse to
                run without an explicit cost ceiling. Long-running Pangram work uses ordinary typed tools, not the
                experimental Tasks extension.
              </p>
              <ul className="pg-list">
                <li>
                  Filesystem paths resolve only inside directories approved with repeated{' '}
                  <code>--allow-file-root PATH</code> at startup.
                </li>
                <li>History reads, history mutations, config changes, and public links each need their own flag.</li>
                <li>
                  <code>pangram mcp install</code> writes client configuration, with <code>--dry-run</code> and a
                  matching uninstall.
                </li>
                <li>An embedded, version-matched skill tells the agent which command or tool to reach for.</li>
              </ul>
              <p className="pg-links">
                <Link href="/docs/reference/mcp-tools" className="pg-link">
                  Tool inventory
                </Link>
                <Link href="/docs/how-to/mcp-gates" className="pg-link">
                  Capability gates
                </Link>
              </p>
            </div>
            <img
              className="pg-band"
              src="/illustrations/agent-archive-1800.webp"
              srcSet="/illustrations/agent-archive-900.webp 900w, /illustrations/agent-archive-1800.webp 1800w"
              sizes="(min-width: 1024px) 700px, 100vw"
              width={1800}
              height={1008}
              alt="Illustration of a vast archive of wooden card-catalog drawers threaded with glass pipes carrying paper strips."
              loading="lazy"
              decoding="async"
            />
          </div>
        </section>

        <section className="pg-shell pg-section">
          <div className="pg-split">
            <div>
              <p className="pg-eyebrow">Failures are contracts too</p>
              <h2 className="pg-heading">Stable codes, categories, and recovery actions.</h2>
              <p className="pg-lede">
                stdout carries results, stderr carries progress and diagnostics. Every failure names a code, a
                category, whether retrying can help, and what to do next. Exit codes stay stable across releases.
              </p>
              <p className="pg-links">
                <Link href="/docs/reference/errors" className="pg-link">
                  Error codes
                </Link>
                <Link href="/docs/reference/exit-codes" className="pg-link">
                  Exit codes
                </Link>
              </p>
            </div>
            <div className="pg-demo">
              <div className="pg-demo__bar">
                <span className="pg-demo__dot" />
                <span className="pg-demo__dot" />
                <span className="pg-demo__dot" />
                <span className="pg-demo__title">exit 4</span>
              </div>
              <div className="pg-demo__body">
                <pre>{errorEnvelope}</pre>
              </div>
            </div>
          </div>
        </section>

        <section className="pg-shell pg-section">
          <p className="pg-eyebrow">Command surface</p>
          <h2 className="pg-heading">What ships today.</h2>
          <div className="pg-fields">
            {commands.map((command) => (
              <div key={command.name} className="pg-fields__row">
                <span className="pg-fields__key">{command.name}</span>
                <span className="pg-fields__value">{command.purpose}</span>
              </div>
            ))}
          </div>
          <p className="pg-small">
            Also available: <code>auth</code>, <code>config</code>, <code>task</code>, <code>agent</code>,{' '}
            <code>skills</code>, <code>completions</code>, and <code>update</code>.{' '}
            <Link href="/docs/reference/commands" className="pg-link">
              Full command index
            </Link>
            .
          </p>
        </section>

        <section className="pg-shell pg-section">
          <div className="pg-split">
            <div>
              <p className="pg-eyebrow">Privacy</p>
              <h2 className="pg-heading">Off by default, and it stays off.</h2>
              <ul className="pg-list">
                <li>Local history is disabled until you enable it, then it stores results locally until you delete them.</li>
                <li>Public Pangram dashboard links are disabled by default and requested per submission.</li>
                <li>Content sent for analysis goes to Pangram and is subject to their retention policy.</li>
                <li>Credentials live in a protected local file, or come from the environment for CI and agents.</li>
              </ul>
              <p className="pg-links">
                <Link href="/docs/explanation/privacy" className="pg-link">
                  Privacy and retention
                </Link>
                <Link href="/docs/how-to/local-history" className="pg-link">
                  Local history
                </Link>
              </p>
            </div>
            <div>
              <p className="pg-eyebrow">Start here</p>
              <ul className="pg-list">
                <li>
                  <Link href="/docs/tutorials/first-tui-analysis" className="pg-link">
                    Run a first analysis in the TUI
                  </Link>
                </li>
                <li>
                  <Link href="/docs/tutorials/first-json-pipeline" className="pg-link">
                    Build a JSON pipeline
                  </Link>
                </li>
                <li>
                  <Link href="/docs/tutorials/first-mcp-client" className="pg-link">
                    Connect an MCP client
                  </Link>
                </li>
              </ul>
            </div>
          </div>
          <img
            className="pg-band"
            src="/illustrations/docs-library-1800.webp"
            srcSet="/illustrations/docs-library-900.webp 900w, /illustrations/docs-library-1800.webp 1800w"
            sizes="(min-width: 1264px) 1200px, 100vw"
            width={1800}
            height={1008}
            alt="Illustration of a modern library with curved balconies where paper strips drift between the shelves."
            loading="lazy"
            decoding="async"
          />
        </section>
      </main>

      <footer className="pg-shell pg-footer">
        <p className="pg-subheading">pangram cli</p>
        <div className="pg-links">
          <Link href="/docs" className="pg-link">
            Documentation
          </Link>
          <Link href="/docs/reference/commands" className="pg-link">
            Commands
          </Link>
          <Link href="/docs/reference/mcp-tools" className="pg-link">
            MCP tools
          </Link>
          <a href="https://github.com/Microck/pangram-cli" className="pg-link">
            GitHub
          </a>
          <a href="https://www.pangram.com/apikey" className="pg-link">
            Pangram API key
          </a>
        </div>
        <p className="pg-footer__note">
          Unofficial and MIT licensed. Not affiliated with, endorsed by, or connected to Pangram Labs, Inc. It is an
          independent client for documented Pangram APIs. Use of Pangram services stays subject to Pangram's terms,
          billing, retention, and acceptable-use policies. Detection results are probabilistic evidence and should be
          one signal among many, not a verdict.
        </p>
      </footer>
    </div>
  );
}
