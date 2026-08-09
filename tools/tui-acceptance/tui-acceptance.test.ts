import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { createServer, type IncomingMessage, type ServerResponse } from "node:http"
import { tmpdir } from "node:os"
import { dirname, isAbsolute, join, resolve } from "node:path"
import { text as consumeText } from "node:stream/consumers"
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
const LOOPBACK_TASK_ID = "task-terminal-control"
const HOSTILE_UPSTREAM_MESSAGE = "synthetic upstream failure\u001b[31mforged\nsecond line"
const ANALYSIS_ID_PATTERN = /anl_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gu
const STABLE_ANALYSIS_ID = "anl_00000000-0000-0000-0000-000000000000"
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

type ScenarioOptions = {
  environment?: Readonly<Record<string, string>>
  prepare?: (fixture: IsolatedFixture) => Promise<void>
}

type LoopbackMode = "success" | "failure" | "polling"

type LoopbackRequest = {
  method: string
  path: string
  body: string
  authenticated: boolean
}

type LoopbackFixture = {
  baseUrl: string
  requests: LoopbackRequest[]
  close: () => Promise<void>
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
    "submits through the shared analyzer and renders a successful result",
    { timeout: 20_000 },
    () =>
      runAnalysisOutcome({
        mode: "success",
        artifactName: "analysis-success",
        text: "This synthetic terminal journey remains human written today",
        expected: ["Overall: succeeded", "Classification: Human", "Human 100.0%"],
      }),
  )

  test(
    "renders a hostile upstream terminal failure without terminal injection",
    { timeout: 20_000 },
    () =>
      runAnalysisOutcome({
        mode: "failure",
        artifactName: "analysis-upstream-failure",
        text: "This synthetic request produces one terminal provider failure",
        expected: ["Overall: failed", "Pangram could not analyze the submitted text."],
        sensitiveValues: [HOSTILE_UPSTREAM_MESSAGE],
      }),
  )

  test(
    "interrupts an indefinitely polling analysis with Ctrl+C",
    { timeout: 20_000 },
    () => runPollingExitJourney("ctrl-c"),
  )

  test(
    "uses the focusable Quit action while an analysis is polling",
    { timeout: 20_000 },
    () => runPollingExitJourney("quit"),
  )

  test(
    "fails closed when local history is not a SQLite database",
    { timeout: 20_000 },
    async () => {
      const corruptBytes = "synthetic protected bytes that are not SQLite"
      await runIsolatedScenario(
        "history-corrupt",
        async (session, fixture) => {
          await session.screen.waitForText("Text composer", { timeoutMs: START_TIMEOUT_MS })
          await session.keyboard.press("Escape")
          await pressMany(session, "ArrowRight", 2)
          await session.screen.waitForText("History unavailable:", {
            timeoutMs: START_TIMEOUT_MS,
          })
          await expectSettledScreen(session, "history-corrupt-fail-closed")
          expect(await readFile(historyDatabasePath(fixture), "utf8")).toBe(corruptBytes)
          await quitNormally(session)
        },
        {
          environment: { PANGRAM_API_KEY: SYNTHETIC_API_KEY },
          prepare: async (fixture) => {
            await writeFile(
              fixture.environment.PANGRAM_CONFIG,
              "config_version = 1\n\n[updates]\ncheck_on_tui_start = false\n",
            )
            const historyDirectory = join(fixture.environment.PANGRAM_DATA_DIR, "history")
            await mkdir(historyDirectory, { recursive: true })
            await chmod(historyDirectory, 0o700)
            const databasePath = historyDatabasePath(fixture)
            await writeFile(databasePath, corruptBytes)
            await chmod(databasePath, 0o600)
          },
        },
      )
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

async function runAnalysisOutcome(options: {
  mode: "success" | "failure"
  artifactName: string
  text: string
  expected: readonly string[]
  sensitiveValues?: readonly string[]
}): Promise<void> {
  await runLoopbackAnalysis(options.mode, options.artifactName, options.text, async (session) => {
    await session.screen.waitUntil(
      ({ text }) => options.expected.every((expected) => text.includes(expected)),
      { timeoutMs: START_TIMEOUT_MS },
    )
    await expectSettledScreen(session, options.artifactName)
    await quitNormally(session, options.sensitiveValues)
  })
}

async function runPollingExitJourney(exit: "ctrl-c" | "quit"): Promise<void> {
  const artifactName = `analysis-polling-${exit}`
  const text = `This synthetic request remains active until ${exit}`
  await runLoopbackAnalysis("polling", artifactName, text, async (session) => {
    await session.screen.waitForText("Stage: STAGE_INFERENCE", { timeoutMs: START_TIMEOUT_MS })
    if (exit === "ctrl-c") {
      await expectSettledScreen(session, artifactName)
      await session.keyboard.press("Control+C")
      await expectCleanExit(session, 130)
      return
    }
    await session.keyboard.press("Escape")
    await session.keyboard.press("ArrowRight")
    await session.screen.waitUntil(
      ({ text: visible }) =>
        visible.includes("1 in-session operation(s)") && visible.includes("Active: 1"),
      { timeoutMs: START_TIMEOUT_MS },
    )
    await expectSettledScreen(session, "active-analysis-polling")
    await quitNormally(session)
  })
}

async function runLoopbackAnalysis(
  mode: LoopbackMode,
  artifactName: string,
  text: string,
  interact: (session: Session) => Promise<void>,
): Promise<void> {
  const loopback = await startLoopbackFixture(mode)
  try {
    await runIsolatedScenario(
      artifactName,
      async (session) => {
        await finishUpdateOnboarding(session)
        await submitAnalysis(session, text)
        await interact(session)
      },
      {
        environment: {
          PANGRAM_API_KEY: SYNTHETIC_API_KEY,
          PANGRAM_DETECT_ENDPOINT: loopback.baseUrl,
        },
      },
    )
    expectOneSubmission(loopback, text)
    expect(
      loopback.requests.some(
        ({ method, path }) => method === "GET" && path === `/task/${LOOPBACK_TASK_ID}`,
      ),
    ).toBe(true)
  } finally {
    await loopback.close()
  }
}

async function startLoopbackFixture(mode: LoopbackMode): Promise<LoopbackFixture> {
  const requests: LoopbackRequest[] = []
  const server = createServer((request, response) => {
    void handleLoopbackRequest(mode, requests, request, response).catch(() => {
      if (!response.headersSent) response.statusCode = 500
      response.end()
    })
  })

  await new Promise<void>((resolveListening, rejectListening) => {
    const handleError = (error: Error) => rejectListening(error)
    server.once("error", handleError)
    server.listen(0, "127.0.0.1", () => {
      server.off("error", handleError)
      resolveListening()
    })
  })
  const address = server.address()
  if (!address || typeof address === "string") {
    server.close()
    throw new Error("loopback fixture did not bind a TCP address")
  }

  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    requests,
    close: () =>
      new Promise<void>((resolveClosed, rejectClosed) => {
        server.close((error) => {
          if (error) rejectClosed(error)
          else resolveClosed()
        })
        server.closeAllConnections()
      }),
  }
}

async function handleLoopbackRequest(
  mode: LoopbackMode,
  requests: LoopbackRequest[],
  request: IncomingMessage,
  response: ServerResponse,
): Promise<void> {
  const body = await readRequestBody(request)
  const method = request.method ?? ""
  const path = request.url ?? ""
  requests.push({
    method,
    path,
    body,
    authenticated: request.headers["x-api-key"] === SYNTHETIC_API_KEY,
  })

  if (method === "POST" && path === "/task") {
    writeJson(response, { task_id: LOOPBACK_TASK_ID })
    return
  }
  if (method === "GET" && path === `/task/${LOOPBACK_TASK_ID}`) {
    if (mode === "success") {
      const submitted = requests.find((candidate) => candidate.method === "POST")
      const submittedText = submitted ? JSON.parse(submitted.body).text : ""
      writeJson(response, pangram4Success(submittedText))
    } else if (mode === "failure") {
      writeJson(response, { stage: "STAGE_FAILED", error_message: HOSTILE_UPSTREAM_MESSAGE })
    } else {
      writeJson(response, { task_id: LOOPBACK_TASK_ID, stage: "STAGE_INFERENCE" })
    }
    return
  }

  response.statusCode = 404
  writeJson(response, { error: "unexpected synthetic fixture route" })
}
async function readRequestBody(request: IncomingMessage): Promise<string> {
  return consumeText(request)
}
function writeJson(response: ServerResponse, value: unknown): void {
  response.setHeader("content-type", "application/json")
  response.end(JSON.stringify(value))
}

function pangram4Success(text: string) {
  const wordCount = text.trim().split(/\s+/u).length
  return {
    stage: "STAGE_SUCCESS",
    text,
    version: "4.0",
    headline: "Human-written",
    prediction: "The document appears to be human-written.",
    prediction_short: "Human",
    fraction_ai: 0,
    fraction_ai_assisted: 0,
    fraction_human: 1,
    num_ai_segments: 0,
    num_ai_assisted_segments: 0,
    num_human_segments: 1,
    windows: [
      {
        text,
        label: "Human Written",
        ai_assistance_score: 0,
        confidence: "High",
        start_index: 0,
        end_index: text.length,
        word_count: wordCount,
        token_length: wordCount,
        is_humanized: false,
        humanizer_score: 0,
      },
    ],
  }
}

function expectOneSubmission(loopback: LoopbackFixture, expectedText: string): void {
  const submissions = loopback.requests.filter((request) => request.method === "POST")
  expect(submissions).toHaveLength(1)
  const [submission] = submissions
  expect(submission.path).toBe("/task")
  expect(submission.authenticated).toBe(true)
  expect(JSON.parse(submission.body)).toMatchObject({
    text: expectedText,
    model: "pangram-4",
    public_dashboard_link: false,
  })
  expect(
    loopback.requests.every((request) =>
      ["/task", `/task/${LOOPBACK_TASK_ID}`].includes(request.path),
    ),
  ).toBe(true)
}
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
  run: (session: Session, fixture: IsolatedFixture) => Promise<void>,
  options: ScenarioOptions = {},
): Promise<void> {
  const { pangramBinary, terminal } = initializedHarness()
  const fixture = await makeIsolatedFixture()
  let session: Session | undefined
  let primaryError: unknown

  try {
    await options.prepare?.(fixture)
    session = await terminal.launch({
      command: [pangramBinary],
      viewport: { cols: 120, rows: 40 },
      color: "never",
      inheritEnv: false,
      env: { ...fixture.environment, ...options.environment },
    })
    await run(session, fixture)
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

function historyDatabasePath(fixture: IsolatedFixture): string {
  return join(fixture.environment.PANGRAM_DATA_DIR, "history", "pangram-history.db")
}

async function finishCredentialFreeOnboarding(session: Session): Promise<void> {
  await session.screen.waitForText("Credential setup", { timeoutMs: START_TIMEOUT_MS })
  await session.keyboard.press("Escape")
  await finishUpdateOnboarding(session)
}

async function finishUpdateOnboarding(session: Session): Promise<void> {
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

async function submitAnalysis(session: Session, text: string): Promise<void> {
  await session.keyboard.type(text)
  await pressMany(session, "Tab", 3)
  await session.screen.waitForText("> [Enter] Submit <", { timeoutMs: START_TIMEOUT_MS })
  await session.keyboard.press("Enter")
}

async function pressMany(session: Session, key: string, count: number): Promise<void> {
  for (let index = 0; index < count; index += 1) {
    await session.keyboard.press(key)
  }
}

async function quitNormally(session: Session, sensitiveValues: readonly string[] = []): Promise<void> {
  // Quit is a focusable command-bar action. End focuses it; Enter activates
  // it after Escape leaves whichever text field currently owns focus. There
  // is intentionally no single-key quit shortcut.
  await session.keyboard.press("Escape")
  await session.keyboard.press("End")
  await session.screen.waitForText("> [Enter] Quit <", { timeoutMs: START_TIMEOUT_MS })
  await session.keyboard.press("Enter")
  await expectCleanExit(session, 0, sensitiveValues)
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

  // Analysis IDs are generated at runtime. The replacement has the same
  // width, so snapshots keep exact cell geometry while remaining repeatable.
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
  return normalizeDynamicText(rows.join("\n"))
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

function normalizeDynamicText(value: string): string {
  return value.replaceAll(ANALYSIS_ID_PATTERN, STABLE_ANALYSIS_ID)
}

function rgb(color: { r: number; g: number; b: number }): string {
  return `#${[color.r, color.g, color.b]
    .map((channel) => channel.toString(16).padStart(2, "0"))
    .join("")}`
}

async function expectCleanExit(
  session: Session,
  exitCode: number,
  sensitiveValues: readonly string[] = [],
): Promise<void> {
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
  for (const sensitiveValue of sensitiveValues) {
    expect(Buffer.from(transcript).includes(Buffer.from(sensitiveValue))).toBe(false)
  }
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
