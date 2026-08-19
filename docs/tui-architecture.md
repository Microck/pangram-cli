# Pangram CLI TUI architecture

Status: approved for implementation

This document owns the TUI detail split from the main architecture
specification. The module boundaries and cross-adapter rules remain in
[architecture-spec.md](architecture-spec.md).

## 13. TUI adapter

### 13.1 Reducer

The TUI uses one state transition boundary:

```text
AppState + AppEvent -> AppState + Effects
```

Effects request analysis, history, update, URL, or terminal actions. Async work
returns typed events through a channel.

Screen code renders state and emits user-intent events. It does not own
networking or SQLite.

### 13.2 State

The root state owns:

- active route
- focus
- composer and file queue
- current analysis identity and progress
- selected history record
- settings draft
- overlays
- terminal size
- intro phase

Derived values such as parent analysis status, responsive layout, and enabled
actions SHOULD be computed from this state instead of copied into additional
mutable fields.

### 13.3 Layout

The wide layout derives three areas from root state:

```text
+-------------+---------------------------+------------------+
| Route rail  | Center workspace          | Inspector        |
| compact fox | input, progress, results  | state and actions|
|             +---------------------------+------------------+
|             | Contextual command bar                       |
+-------------+----------------------------------------------+
```

The center workspace receives most available width. The inspector contains
checks, privacy, cost, result filters, and actions that apply to the active
state. The route rail contains only the compact resolved mark and primary
routes. It is not an intro canvas.

At narrower supported sizes, navigation becomes top tabs and inspector content
joins the center flow. Layout is derived from terminal size and current state;
wide and narrow screens do not maintain separate mutable UI models.

Rendering uses sparse separators, focus markers, and a restrained command bar.
It does not reproduce web-style cards, oversized buttons, or a persistent
large brand mark. At narrower sizes, the pre-submission Analyze view
bottom-aligns one derived dock that owns selectors, composer, privacy controls,
estimate, and primary action instead of also rendering a detached inspector.
Wide mode retains the dedicated inspector. Shared layout helpers derive the same two-column workspace inset,
top breathing row, dock geometry, modal padding, and control coordinates for
both rendering and pointer hit testing, so visual targets and mouse targets
cannot drift. The wide route rail and separator derive their full height from
the terminal, while workspace and inspector stop above the command band. The
wide command row begins at the workspace edge, then applies that same content
inset; narrow mode derives the row from the full terminal width.

Mouse decoding stays inside the TUI adapter. Screen coordinates become typed
pointer intent before they enter the reducer. The reducer reuses the same
route, focus, activation, list-selection, result-navigation, confirmation, and
operation-gate paths as keyboard intent; raw coordinates never enter
application state.

### 13.4 Intro renderer

The intro is an internal deep module in the TUI adapter. It has no trait and no
public interface. Callers provide resolved intro policy, terminal capabilities,
whether the one-time state has been seen, and monotonic elapsed time. The
module returns either no intro or the frame to render.

The implementation uses one generated 72x16 terminal-cell table:

- 14 resampled source-cycle frames
- eight final dissolve frames
- one 56-entry playback index sequence

The table uses a 20 fps timeline. A development-only generator verifies and
decodes the approved GIF, then quantizes it into styled terminal cells.
Generated tables are committed so normal builds and runtime startup need no
GIF decoder, floating-point rasterizer, or filesystem asset lookup.

The generator samples one sequence:

- resample the nine-frame, 630 ms source cycle into 14 frames
- repeat that cycle four times
- replace playback frames 48 through 55 with deterministic density dissolves

For elapsed time below 2,800 ms, frame selection uses
`floor(elapsed_ms / 50)`. During the first 900 ms, the renderer moves the full
frame backdrop from terminal black to the resolved TUI canvas color. From 2,800
through 3,099 ms, six fixed samples of `cubic-bezier(0.23, 1, 0.32, 1)` blend
the normally rendered Analyze buffer from the canvas color to its final colors.
At 3,100 ms or later, the module returns completion. The event loop does not
enqueue one event per missed frame. This keeps the fox sequence at 2.8 seconds
and the complete presentation at 3.1 seconds when terminal rendering stalls.

The fade post-processes the canonical Analyze buffer. It does not duplicate
widgets, layout, state, or input behavior in a transition-only renderer.

Color capability is resolved once before playback. Fallback selection is
deterministic:

1. truecolor, ANSI approximation, or no-color styling
2. Unicode density glyphs for colored output or ASCII `.`, `+`, `#`, and space
   for `NO_COLOR`

`Escape`, `Enter`, and `Space` produce one skip event that transitions directly
to the final Analyze screen from either presentation phase. The input event is
consumed before normal routing. A resize also reveals the final screen at once.

For `tui.intro = "once"`, the TUI atomically writes its `intro_seen` marker
after completion or explicit skip. The marker lives in TUI state
at `PANGRAM_DATA_DIR/tui-state.json`, not in configuration. A failed write
reports a non-blocking diagnostic and does not stop Analyze. Suppressed startup
does not write it.

The approved source and generated derivatives remain outside the repository's
MIT grant and carry their separate artwork notice. Third-party source, frames,
and marketing assets MUST NOT enter the repository.

### 13.5 Terminal lifecycle

One terminal guard owns:

- raw mode
- alternate screen and bracketed-paste mode
- cursor visibility
- mouse capture when enabled
- panic restoration

The guard's restoration operation is idempotent and restores state on drop.
Process-level unwind panic handling and supported signal handling invoke the
same operation before printing diagnostics. Guarded code returns exit intent
and MUST NOT call `process::exit`. The build uses `panic = "unwind"`.

The release guarantee covers normal return, handled I/O failure, Ctrl+C,
supported catchable termination signals, and unwind panic. Process abort,
SIGKILL, and equivalent uncatchable termination are outside the guarantee.

### 13.6 Autonomous acceptance boundary

The compiled TUI is exercised through a development-only Terminal Control
harness. Product-only journeys launch the real `pangram` binary in a real PTY.
Protocol journeys launch `pangram-test-driver`, which injects a loopback
analyzer and then enters the exact same TUI adapter. Both paths drive the same
keyboard and resize handling a person uses. Neither the harness nor the test
driver ships in release artifacts.

The harness uses `@kitlangton/terminal-control 0.6.0` with Vitest. Tests set an
isolated config directory, data directory, home directory, locale, terminal
type, and viewport. They do not inherit the operator's environment. Settled
text and cell frames are source-controlled snapshots. Text, JSON, SVG, logs,
and metadata are retained as failure evidence. PNG captures and recordings are
opt-in review artifacts and use synthetic, credential-free scenarios because
typed input and terminal streams may contain secrets.

The acceptance harness runs on GNU/Linux and macOS, matching Terminal Control's
published native packages. Windows keeps its native platform PTY and terminal
restoration tests. A platform-independent Ratatui `TestBackend` layer remains
the source of deterministic renderer snapshots.

Do not implement Terminal Control's optional OpenTUI semantic protocol. It is
not a Ratatui contract, and adding it would create a second machine-only UI
surface that could pass while the visible terminal is broken.
