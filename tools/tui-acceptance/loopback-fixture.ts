import { createServer, type IncomingMessage, type ServerResponse } from "node:http"
import { text as consumeText } from "node:stream/consumers"

export const LOOPBACK_TASK_ID = "task-terminal-control"
export const HOSTILE_UPSTREAM_MESSAGE = "synthetic upstream failure\u001b[31mforged\nsecond line"

export type LoopbackMode = "success" | "failure" | "polling"

export type LoopbackRequest = {
  method: string
  path: string
  body: string
  authenticated: boolean
}

export type LoopbackFixture = {
  baseUrl: string
  requests: LoopbackRequest[]
  close: () => Promise<void>
}

export async function startLoopbackFixture(
  mode: LoopbackMode,
  apiKey: string,
): Promise<LoopbackFixture> {
  const requests: LoopbackRequest[] = []
  const server = createServer((request, response) => {
    void handleLoopbackRequest(mode, apiKey, requests, request, response).catch(() => {
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
  apiKey: string,
  requests: LoopbackRequest[],
  request: IncomingMessage,
  response: ServerResponse,
): Promise<void> {
  const method = request.method ?? ""
  const path = request.url ?? ""
  const body = method === "POST" ? await consumeText(request) : ""
  requests.push({
    method,
    path,
    body,
    authenticated: request.headers["x-api-key"] === apiKey,
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
