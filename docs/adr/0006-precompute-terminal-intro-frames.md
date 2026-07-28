# ADR 0006: Precompute terminal intro frames

Status: accepted
Date: 2026-07-23

## Context

The Pangram TUI needs a short launch intro based on the supplied fox-mark
motion reference. The reference is a 1280x720, 60 fps media export. A terminal
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
- 2.8 seconds nominal duration
- 32x16 full geometry
- 20x10 compact geometry
- truecolor, ANSI, no-color, and ASCII presentation paths

A development-only Rust generator converts approved fox vector geometry into
the constant tables. Normal builds and runtime playback use only the generated
tables.

The geometry, provenance, rights, and acceptance artifact are normative in
`../intro-art-contract.md`. Missing geometry blocks this module rather than the
core TUI.

Playback selects a frame from monotonic elapsed time. It skips stale frames
when rendering falls behind. Escape, Enter, and Space skip playback and are
consumed.

Intro frequency is `once`, `always`, or `off`, with `once` as the default.
Motion is an independent `full`, `reduced`, or `off` setting. The one-time
marker is local state and does not rewrite user configuration.

## Consequences

- Playback has no video decoder, filesystem asset lookup, terminal image
  protocol, or frame-timer backlog.
- Snapshot tests can cover exact phase boundaries and fallbacks.
- Generated frame tables add repository size but little runtime complexity.
- Any geometry or timing change requires regeneration and agent-owned baseline
  acceptance.
- Public distribution remains blocked until Pangram grants written permission
  for terminal use of the fox logo and derived frame art.

## Rejected alternatives

### Bundle the supplied video

This adds decoding and terminal compatibility work for a one-time 2.8-second
sequence.

### Rasterize vector geometry at runtime

This adds floating-point drawing behavior to startup and makes output harder to
test across targets.

### Copy Droid's frame sequence

Droid is useful architecture evidence, not a source asset. Copying its
unlicensed runtime data is unnecessary and unacceptable.

### Procedurally reveal the `PANGRAM` wordmark

This conflicts with the supplied fox reference and creates a second motion
concept without a product need.
