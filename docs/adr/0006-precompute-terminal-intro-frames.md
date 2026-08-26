# ADR 0006: Precompute terminal intro frames

Status: accepted
Date: 2026-08-26

## Context

The Pangram TUI needs a short launch intro based on the approved fox-mark GIF.
The source is a 1772x709, nine-frame animation with a 630 ms cycle. A terminal
cannot reproduce those pixels directly without shipping a media decoder,
depending on terminal-specific image protocols, or adding a runtime rasterizer.

Research into Droid's current CLI found a simpler terminal-native pattern: its
runtime selects from a fixed frame table by elapsed time, then renders centered
text. The Pangram design may use that architecture, but it must not copy
Droid's frames, source, or artwork.

## Decision

Generate and commit original Pangram terminal-cell frame tables:

- 56 frames
- 20 frames per second
- 2.8-second fox sequence followed by a 300 ms Analyze fade
- 72x16 geometry
- truecolor, ANSI, no-color, and ASCII presentation paths

A development-only Rust generator verifies and decodes the approved GIF,
resamples one cycle into 14 terminal frames, and generates eight final dissolve
frames. Normal builds and runtime playback use only those constant tables. The
56-frame sequence repeats the cycle four times and replaces its final eight
frames with the dissolve. It ends after four cycles rather than looping forever.
During the first 900 ms, its backdrop reaches the TUI canvas color. The real
Analyze buffer then fades in over six fixed 50 ms easing samples, for a 3.1
second complete presentation.

The source hash, geometry, provenance, rights, and acceptance artifact are
normative in `../intro-art-contract.md`. Missing or invalid generated art blocks
only the intro rather than the core TUI.

Playback selects a frame from monotonic elapsed time. It skips stale frames
when rendering falls behind. Escape, Enter, and Space skip playback and are
consumed. The transition post-processes the normal Analyze buffer rather than
maintaining a separate transition layout.

Intro frequency is `once`, `always`, or `off`, with `once` as the default.
Motion is an independent `full`, `reduced`, or `off` setting. The one-time
marker is local state and does not rewrite user configuration.

## Consequences

- Playback has no video decoder, filesystem asset lookup, terminal image
  protocol, or frame-timer backlog.
- Snapshot tests can cover exact phase boundaries and fallbacks.
- The intro hands off through the same canvas color as the TUI, without a hard
  background or content cut.
- Generated frame tables add repository size but little runtime complexity.
- Any geometry or timing change requires regeneration and agent-owned baseline
  acceptance.
- The source and generated derivatives remain outside the repository's MIT
  grant and require their separate artwork notice in public distributions.

## Rejected alternatives

### Bundle the supplied video

This adds decoding and terminal compatibility work for a one-time 2.8-second
fox sequence.

### Rasterize source geometry at runtime

This adds floating-point drawing behavior to startup and makes output harder to
test across targets.

### Copy Droid's frame sequence

Droid is useful architecture evidence, not a source asset. Copying its
unlicensed runtime data is unnecessary and unacceptable.

### Procedurally reveal the `PANGRAM` wordmark

This conflicts with the supplied fox reference and creates a second motion
concept without a product need.
