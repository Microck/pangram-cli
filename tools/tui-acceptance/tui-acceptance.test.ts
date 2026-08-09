import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, isAbsolute, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import {
  TerminalControl,
  type ArtifactManifest,
  type Frame,
  type Session,
} from "@kitlangton/terminal-control"
import { afterAll, beforeAll, describe, expect, test } from "vitest"

const START_TIMEOUT_MS = 5_000
const EXIT_TIMEOUT_MS = 5_000
const SYNTHETIC_API_KEY = "synthetic-api-key-not-a-secret"
const SUPPORTED_PLATFORM = process.platform === "linux" || process.platform === "darwin"
const ACCEPTANCE_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const ARTIFACT_DIRECTORY = join(ACCEPTANCE_DIRECTORY, ".artifacts")

type IsolatedFixture = {
  root: string
  environment: Readonly<Record<string, string>>
}

type AcceptanceHarness = {
  pangramBinary: string
  terminal: TerminalControl
}

type SanitizedArtifactManifest = Pick<
  ArtifactManifest,
  "screenText" | "screenFrame" | "screenSvg" | "metadata" | "logsText"
>

type ProjectedRun = {
  x: number
  y: number
  width: number
  text: string
  style: string
}

let harness: AcceptanceHarness | undefined

describe.runIf(SUPPORTED_PLATFORM)("compiled TUI acceptance", () => {
  beforeAll(async () => {
    const configuredBinary = process.env.PANGRAM_BIN
    if (!configuredBinary) {
      throw new Error("PANGRAM_BIN must point to the compiled pangram binary")
    }

    harness = {
      pangramBinary: isAbsolute(configuredBinary) ? configuredBinary : resolve(configuredBinary),
      terminal: await TerminalControl.make({
        artifacts: {
          directory: ARTIFACT_DIRECTORY,
          onFailure: true,
          includeTranscript: false,
          includeRecording: false,
        },
      }),
    }
  })

  afterAll(async () => {
    if (harness) await harness.terminal.close()
  })

  test(
    "shows first-use state and restores the terminal after Ctrl+C",
    { timeout: 20_000 },
    async () => {
      await runIsolatedScenario("first-use-ctrl-c", async (session) => {
        await session.screen.waitForText("Credential setup", { timeoutMs: START_TIMEOUT_MS })
        await expectSettledScreen(session, "first-use-wide")

        await session.keyboard.press("Control+C")
        await expectCleanExit(session, 130)
      })
    },
  )

  test(
    "stores a synthetic credential without exposing it and quits normally",
    { timeout: 20_000 },
    async () => {
      await runIsolatedScenario("stored-authentication", async (session) => {
        await session.screen.waitForText("Credential setup", { timeoutMs: START_TIMEOUT_MS })
        await session.keyboard.type(SYNTHETIC_API_KEY)
        await session.screen.waitUntil(
          ({ text }) => text.includes("API key: ******** (masked)") && !text.includes(SYNTHETIC_API_KEY),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await session.keyboard.press("Enter")
        await session.screen.waitForText("Update checks", { timeoutMs: START_TIMEOUT_MS })
        await session.keyboard.type("n")
        await session.screen.waitForText("Type or paste text here", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await expectSettledScreen(session, "stored-authentication-analyze")

        await quitNormally(session)
      })
    },
  )

  test(
    "skips authentication, navigates, covers responsive boundaries, and quits normally",
    { timeout: 20_000 },
    async () => {
      await runIsolatedScenario("navigation-resize-quit", async (session) => {
        await finishCredentialFreeOnboarding(session)
        await expectSettledScreen(session, "analyze-wide")

        // Escape leaves the composer and focuses the route rail. Right then
        // drives the same regular-keymap route changes a person uses.
        await session.keyboard.press("Escape")
        await session.screen.waitUntil(
          ({ text }) => !text.includes("> Text composer"),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await session.keyboard.press("ArrowRight")
        await session.screen.waitForText("No active analyses in this session.", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await session.keyboard.press("ArrowRight")
        await session.screen.waitForText("Local Pangram CLI history", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await session.keyboard.press("ArrowRight")
        await session.screen.waitForText("Keymap: Regular", { timeoutMs: START_TIMEOUT_MS })

        await session.resize({ cols: 79, rows: 23 })
        await session.screen.waitUntil(
          ({ text }) =>
            text.includes("Terminal too small") && text.includes("Current size: 79x23"),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await expectSettledScreen(session, "settings-below-minimum")

        await session.resize({ cols: 80, rows: 24 })
        await session.screen.waitUntil(
          ({ text }) => text.includes("Keymap: Regular") && !text.includes("Terminal too small"),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await expectSettledScreen(session, "settings-minimum")

        await session.resize({ cols: 100, rows: 30 })
        await session.screen.waitUntil(
          ({ text }) => text.includes("Keymap: Regular") && !text.includes("Terminal too small"),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await expectSettledScreen(session, "settings-narrow")

        await session.resize({ cols: 120, rows: 40 })
        await session.screen.waitUntil(
          ({ text }) => text.includes("Keymap: Regular") && !text.includes("Terminal too small"),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await expectSettledScreen(session, "settings-wide-restored")

        await quitNormally(session)
      })
    },
  )

  test(
    "keeps printable input in the composer under regular and Vim keymaps",
    { timeout: 20_000 },
    async () => {
      await runIsolatedScenario("regular-vim-input-help", async (session) => {
        await finishCredentialFreeOnboarding(session)

        const regularInput = "regular hjkl ? /"
        await session.keyboard.type(regularInput)
        await session.screen.waitUntil(
          ({ text }) => text.includes(regularInput) && text.includes("Words: 4"),
          { timeoutMs: START_TIMEOUT_MS },
        )

        // Switch keymaps through the same persisted Settings action a person
        // uses. Vim route keys must only take effect after focus leaves a text
        // field.
        await session.keyboard.press("Escape")
        await pressMany(session, "ArrowRight", 3)
        await session.screen.waitForText("Keymap: Regular", { timeoutMs: START_TIMEOUT_MS })
        await pressMany(session, "ArrowDown", 4)
        await session.screen.waitForText("> Keymap: Regular", { timeoutMs: START_TIMEOUT_MS })
        await session.keyboard.press("Enter")
        await session.screen.waitForText("> Keymap: Vim", { timeoutMs: START_TIMEOUT_MS })

        await session.keyboard.press("Escape")
        await session.keyboard.type("hhh")
        await session.screen.waitForText("Text composer", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await session.keyboard.press("Home")
        await session.screen.waitForText("> Text composer", { timeoutMs: START_TIMEOUT_MS })

        const vimInput = " vim hjkl ? / gg G n N"
        await session.keyboard.type(vimInput)
        await session.screen.waitUntil(
          ({ text }) =>
            text.includes(`${regularInput}${vimInput}`) && text.includes("Words: 12"),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await expectSettledScreen(session, "vim-printable-composer")

        // Help is keyboard-reachable, dismissible by the same key, and leaves
        // route focus intact for Vim navigation.
        await session.keyboard.press("Escape")
        await session.keyboard.type("?")
        await session.screen.waitForText("Focus Quit in the command bar", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await expectSettledScreen(session, "help-overlay")
        await session.keyboard.type("?")
        await session.screen.waitUntil(
          ({ text }) => !text.includes("Focus Quit in the command bar"),
          { timeoutMs: START_TIMEOUT_MS },
        )

        await session.keyboard.type("l")
        await session.screen.waitForText("No active analyses in this session.", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await session.keyboard.type("h")
        await session.screen.waitForText("Text composer", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await quitNormally(session)
      })
    },
  )

  test(
    "requires visible plaintext consent before enabling local history",
    { timeout: 20_000 },
    async () => {
      await runIsolatedScenario("history-consent", async (session) => {
        await finishCredentialFreeOnboarding(session)
        await session.keyboard.press("Escape")
        await pressMany(session, "ArrowRight", 3)
        await pressMany(session, "ArrowDown", 2)
        await session.screen.waitForText("> History: disabled", {
          timeoutMs: START_TIMEOUT_MS,
        })

        await session.keyboard.press("Enter")
        await session.screen.waitUntil(
          ({ text }) =>
            text.includes("Enable local history") &&
            text.includes("History stores full input and results in plaintext."),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await expectSettledScreen(session, "history-consent-warning")

        await session.keyboard.press("Escape")
        await session.screen.waitUntil(
          ({ text }) =>
            text.includes("> History: disabled") && !text.includes("Enable local history"),
          { timeoutMs: START_TIMEOUT_MS },
        )

        await session.keyboard.press("Enter")
        await session.screen.waitForText("Enable local history", { timeoutMs: START_TIMEOUT_MS })
        await session.keyboard.type("y")
        await session.screen.waitForText("> History: enabled", {
          timeoutMs: START_TIMEOUT_MS,
        })

        // Disabling is not destructive and must not show the retention
        // warning again.
        await session.keyboard.press("Enter")
        await session.screen.waitUntil(
          ({ text }) =>
            text.includes("> History: disabled") && !text.includes("Enable local history"),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await quitNormally(session)
      })
    },
  )

  test(
    "filters empty history and keeps export confirmations cancel-safe",
    { timeout: 20_000 },
    async () => {
      await runIsolatedScenario("history-export-confirmations", async (session) => {
        await finishCredentialFreeOnboarding(session)
        await session.keyboard.press("Escape")
        await pressMany(session, "ArrowRight", 2)
        await session.screen.waitForText("No saved analyses match these criteria.", {
          timeoutMs: START_TIMEOUT_MS,
        })

        await session.keyboard.press("ArrowDown")
        await session.keyboard.type("needle hjkl")
        await session.screen.waitForText("> Search literal: needle hjkl", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await session.keyboard.press("Enter")
        await session.keyboard.press("ArrowDown")
        await session.keyboard.press("Enter")
        await session.screen.waitForText("> Status filter: queued", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await session.keyboard.press("ArrowDown")
        await session.keyboard.press("Enter")
        await session.screen.waitForText("> Check filter: AI detection", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await expectSettledScreen(session, "history-filtered-empty")

        // Down moves selection while the list owns focus. Tab traverses the
        // focus order and therefore reaches the contextual Export action.
        await pressMany(session, "Tab", 3)
        await session.screen.waitForText(">Export<", { timeoutMs: START_TIMEOUT_MS })
        await session.keyboard.press("Enter")
        await session.screen.waitUntil(
          ({ text }) =>
            text.includes("Export local history") &&
            text.includes("Format: JSONL") &&
            text.includes("Content: redacted") &&
            text.includes("> Action: cancel <"),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await expectSettledScreen(session, "history-export-default-cancel")

        // Enter on the default action cancels and writes no stdout document.
        await session.keyboard.press("Enter")
        await session.screen.waitUntil(
          ({ text }) => text.includes(">Export<") && !text.includes("Export local history"),
          { timeoutMs: START_TIMEOUT_MS },
        )

        // Choosing full content and Export must add a second confirmation.
        // That confirmation also focuses Cancel, so a bare Enter stays safe.
        await session.keyboard.press("Enter")
        await session.keyboard.press("ArrowUp")
        await session.keyboard.press("ArrowRight")
        await session.screen.waitForText("> Content: full retained content <", {
          timeoutMs: START_TIMEOUT_MS,
        })
        await session.keyboard.press("ArrowDown")
        await session.keyboard.press("ArrowRight")
        await session.keyboard.press("Enter")
        await session.screen.waitUntil(
          ({ text }) =>
            text.includes("Export full retained content") &&
            text.includes("> [Enter] Cancel <") &&
            text.includes("[Right] Export full content"),
          { timeoutMs: START_TIMEOUT_MS },
        )
        await expectSettledScreen(session, "history-full-export-confirmation")
        await session.keyboard.press("Enter")
        await session.screen.waitUntil(
          ({ text }) => text.includes(">Export<") && !text.includes("Export full retained content"),
          { timeoutMs: START_TIMEOUT_MS },
        )

        await quitNormally(session)
      })
    },
  )
})

test("failure artifacts redact fixture and binary paths", async () => {
  const root = await mkdtemp(join(tmpdir(), "pangram-tui-artifact-redaction-"))
  const sensitivePath = join(root, "private", "pangram")
  const artifactPaths = {
    screenText: join(root, "screen.txt"),
    screenFrame: join(root, "screen.json"),
    screenSvg: join(root, "screen.svg"),
    metadata: join(root, "metadata.json"),
    logsText: join(root, "logs.txt"),
  }

  try {
    await Promise.all(
      Object.values(artifactPaths).map((path) =>
        writeFile(
          path,
          `before ${sensitivePath} secret ${SYNTHETIC_API_KEY} after ${sensitivePath}`,
        ),
      ),
    )
    await sanitizeFailureArtifacts(artifactPaths, [sensitivePath], [SYNTHETIC_API_KEY])

    for (const path of Object.values(artifactPaths)) {
      const contents = await readFile(path, "utf8")
      expect(contents).not.toContain(sensitivePath)
      expect(contents).not.toContain(SYNTHETIC_API_KEY)
      expect(contents).toBe(
        "before [redacted-path] secret [redacted-value] after [redacted-path]",
      )
    }
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

async function runIsolatedScenario(
  artifactName: string,
  run: (session: Session) => Promise<void>,
): Promise<void> {
  const { pangramBinary, terminal } = initializedHarness()
  const fixture = await makeIsolatedFixture()
  let session: Session | undefined
  let primaryError: unknown

  try {
    session = await terminal.launch({
      command: [pangramBinary],
      viewport: { cols: 120, rows: 40 },
      color: "never",
      inheritEnv: false,
      env: fixture.environment,
    })
    await run(session)
  } catch (error) {
    primaryError = error
    if (session) {
      try {
        const manifest = await session.writeArtifacts(artifactName)
        await sanitizeFailureArtifacts(
          manifest,
          [fixture.root, pangramBinary],
          [SYNTHETIC_API_KEY],
        )
        console.error(`Terminal Control failure artifacts: ${manifest.directory}`)
      } catch (artifactError) {
        console.error("Could not write Terminal Control failure artifacts", artifactError)
      }
    }
    throw error
  } finally {
    let cleanupError: unknown
    try {
      if (session) await session.stop()
    } catch (error) {
      cleanupError = error
      console.error("Could not stop Terminal Control session", error)
    } finally {
      await rm(fixture.root, { recursive: true, force: true })
    }
    if (!primaryError && cleanupError) throw cleanupError
  }
}

function initializedHarness(): AcceptanceHarness {
  if (!harness) throw new Error("TUI acceptance harness was not initialized")
  return harness
}

async function makeIsolatedFixture(): Promise<IsolatedFixture> {
  const root = await mkdtemp(join(tmpdir(), "pangram-tui-acceptance-"))
  const home = join(root, "home")
  const configDirectory = join(root, "config")
  const dataDirectory = join(root, "data")
  const temporaryDirectory = join(root, "tmp")
  await Promise.all(
    [home, configDirectory, dataDirectory, temporaryDirectory].map((directory) =>
      mkdir(directory, { recursive: true }),
    ),
  )

  return {
    root,
    environment: {
      HOME: home,
      USERPROFILE: home,
      XDG_CONFIG_HOME: configDirectory,
      XDG_DATA_HOME: dataDirectory,
      PANGRAM_CONFIG: join(configDirectory, "config.toml"),
      PANGRAM_DATA_DIR: dataDirectory,
      TMPDIR: temporaryDirectory,
      TERM: "xterm-256color",
      COLORTERM: "truecolor",
      LANG: "C.UTF-8",
      LC_ALL: "C.UTF-8",
      TZ: "UTC",
      CI: "true",
      NO_COLOR: "1",
    },
  }
}

async function finishCredentialFreeOnboarding(session: Session): Promise<void> {
  await session.screen.waitForText("Credential setup", { timeoutMs: START_TIMEOUT_MS })
  await session.keyboard.press("Escape")
  await session.screen.waitForText("Update checks", { timeoutMs: START_TIMEOUT_MS })
  await session.keyboard.type("n")
  await session.screen.waitUntil(
    ({ text }) =>
      text.includes("Type or paste text here") &&
      text.includes("[Analyze]") &&
      !text.includes("Update checks"),
    { timeoutMs: START_TIMEOUT_MS },
  )
}

async function pressMany(session: Session, key: string, count: number): Promise<void> {
  for (let index = 0; index < count; index += 1) {
    await session.keyboard.press(key)
  }
}

async function quitNormally(session: Session): Promise<void> {
  // Quit is a focusable command-bar action. End focuses it; Enter activates
  // it after Escape leaves whichever text field currently owns focus. There
  // is intentionally no single-key quit shortcut.
  await session.keyboard.press("Escape")
  await session.keyboard.press("End")
  await session.screen.waitForText("> [Enter] Quit <", { timeoutMs: START_TIMEOUT_MS })
  await session.keyboard.press("Enter")
  await expectCleanExit(session, 0)
}

async function expectSettledScreen(session: Session, snapshotName: string): Promise<void> {
  const capture = await session.screen.capture({ settleMs: 30, deadlineMs: START_TIMEOUT_MS })
  expect(capture.reason).toBe("idle")
  expect(projectSettledScreen(capture.text, capture.frame)).toMatchSnapshot(
    `${snapshotName} screen`,
  )
}

function projectSettledScreen(screenText: string, frame: Frame): string {
  const { cells, ...geometry } = frame
  const visibleCells = cells.filter((cell) => isVisibleCell(cell, frame))
  const runs: ProjectedRun[] = []

  // Adjacent cells with the same style carry one visual fact. Folding them
  // into exact-position runs keeps cell-level coverage while leaving a
  // snapshot a reviewer can inspect instead of tens of thousands of lines.
  for (const cell of visibleCells) {
    const projected = projectCell(cell)
    const previous = runs.at(-1)
    if (
      previous &&
      previous.y === projected.y &&
      previous.x + previous.width === projected.x &&
      previous.style === projected.style
    ) {
      previous.text += projected.text
      previous.width += projected.width
    } else {
      runs.push(projected)
    }
  }

  const textRows = screenText.split("\n")
  const rows = [JSON.stringify(geometry)]
  for (let y = 0; y < frame.rows; y += 1) {
    const rowRuns = runs
      .filter((run) => run.y === y)
      .map(({ x, width, text, style }) => [x, width, text, style])
    rows.push(
      `${y.toString().padStart(2, "0")} text=${JSON.stringify(textRows[y] ?? "")} cells=${JSON.stringify(rowRuns)}`,
    )
  }
  return rows.join("\n")
}

function isVisibleCell(cell: Frame["cells"][number], frame: Frame): boolean {
  return (
    cell.text.trim() !== "" ||
    cell.background.r !== frame.background.r ||
    cell.background.g !== frame.background.g ||
    cell.background.b !== frame.background.b ||
    cell.attributes.bold ||
    cell.attributes.italic ||
    cell.attributes.faint ||
    cell.attributes.invisible ||
    cell.attributes.strikethrough ||
    cell.attributes.overline ||
    cell.attributes.underline !== null
  )
}

function projectCell(cell: Frame["cells"][number]): ProjectedRun {
  const attributes = Object.entries(cell.attributes)
    .filter(([, value]) => value !== false && value !== null)
    .map(([name, value]) => (value === true ? name : `${name}:${JSON.stringify(value)}`))
    .join(",")
  const style = [rgb(cell.foreground), rgb(cell.background), attributes]
    .filter((value) => value !== "")
    .join(" ")
  return { x: cell.x, y: cell.y, width: cell.width, text: cell.text, style }
}

function rgb(color: { r: number; g: number; b: number }): string {
  return `#${[color.r, color.g, color.b]
    .map((channel) => channel.toString(16).padStart(2, "0"))
    .join("")}`
}

async function expectCleanExit(session: Session, exitCode: number): Promise<void> {
  const completed = await session.waitForExit({ timeoutMs: EXIT_TIMEOUT_MS })
  expect(completed).toEqual({
    reason: "exited",
    exit: {
      code: exitCode,
      signal: null,
      success: exitCode === 0,
    },
  })

  const transcript = await session.transcript.ansi()
  expect(transcript).toContainBytes("\u001b[?1049h")
  expect(transcript).toContainBytes("\u001b[?1049l")
  expect(transcript).toContainBytes("\u001b[?25h")
  expect(Buffer.from(transcript).includes(Buffer.from(SYNTHETIC_API_KEY))).toBe(false)
}

async function sanitizeFailureArtifacts(
  manifest: SanitizedArtifactManifest,
  sensitivePaths: readonly string[],
  sensitiveValues: readonly string[] = [],
): Promise<void> {
  const artifactFiles = [
    manifest.screenText,
    manifest.screenFrame,
    manifest.screenSvg,
    manifest.metadata,
    manifest.logsText,
  ]
  await Promise.all(
    artifactFiles.map(async (path) => {
      let contents = await readFile(path, "utf8")
      for (const sensitivePath of sensitivePaths) {
        contents = contents.replaceAll(sensitivePath, "[redacted-path]")
      }
      for (const sensitiveValue of sensitiveValues) {
        contents = contents.replaceAll(sensitiveValue, "[redacted-value]")
      }
      await writeFile(path, contents)
    }),
  )
}

declare module "vitest" {
  interface Assertion<T> {
    toContainBytes(expected: string): void
  }
}

expect.extend({
  toContainBytes(received: Uint8Array, expected: string) {
    const pass = Buffer.from(received).includes(Buffer.from(expected, "utf8"))
    return {
      pass,
      message: () =>
        pass
          ? `expected terminal transcript not to contain ${JSON.stringify(expected)}`
          : `expected terminal transcript to contain ${JSON.stringify(expected)}`,
    }
  },
})
