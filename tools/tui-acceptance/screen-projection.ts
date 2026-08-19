import type { Frame } from "@kitlangton/terminal-control"

const ANALYSIS_ID_PATTERN =
  /anl_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gu
const STABLE_ANALYSIS_ID = "anl_00000000-0000-0000-0000-000000000000"
const RFC3339_SUBSECOND_PATTERN =
  /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{1,9}Z/gu
const STABLE_TIMESTAMP = "[timestamp]"
const SGR_PATTERN = /\u001b\[[0-9:;]*m/gu

type ProjectedRun = {
  x: number
  y: number
  width: number
  text: string
  style: string
}

export function projectSettledScreen(frame: Frame): string {
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

  for (const run of runs) {
    const normalizedText = normalizeDynamicText(run.text)
    if (normalizedText !== run.text) {
      run.text = normalizedText
      run.width = normalizedText.length
    }
  }

  // Analysis IDs and timestamps are generated at runtime. Normalizing their
  // visible runs keeps fixed positions and styles reviewable without making
  // snapshots depend on one process clock.
  // The structured frame is the authoritative settled screen. Terminal
  // Control's convenience text can briefly retain an erased blank from a
  // delta repaint even when the corresponding cells already agree.
  const textRows = frameTextRows(frame)
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

function frameTextRows(frame: Frame): string[] {
  const cellsByRow = Array.from({ length: frame.rows }, () => [] as Frame["cells"])
  for (const cell of frame.cells) {
    cellsByRow[cell.y]?.push(cell)
  }

  return cellsByRow.map((cells) => {
    let column = 0
    let text = ""
    for (const cell of cells.sort((left, right) => left.x - right.x)) {
      // A wide glyph can have continuation cells inside its occupied width.
      if (cell.x < column) continue
      text += " ".repeat(cell.x - column) + cell.text
      column = cell.x + cell.width
    }
    return normalizeScreenText(text.trimEnd())
  })
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
  return value
    .replaceAll(ANALYSIS_ID_PATTERN, STABLE_ANALYSIS_ID)
    .replaceAll(RFC3339_SUBSECOND_PATTERN, STABLE_TIMESTAMP)
}

export function normalizeScreenText(value: string): string {
  // Terminal Control can encode the live cursor as reverse-video SGR bytes in
  // convenience text even when the authoritative structured cells are stable.
  return normalizeDynamicText(value)
    .replaceAll(SGR_PATTERN, "")
    .replace(/(\[timestamp\]) +(?=\S)/gu, "$1 ")
}

function rgb(color: { r: number; g: number; b: number }): string {
  return `#${[color.r, color.g, color.b]
    .map((channel) => channel.toString(16).padStart(2, "0"))
    .join("")}`
}
