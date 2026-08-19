# Pangram CLI intro art contract

Status: source approved and locked

This contract owns the source, provenance, license boundary, and acceptance
evidence for the generated terminal fox frames. Intro behavior and timing remain
owned by the product contract and ADR 0006.

## Required source

The development tree contains the exact approved source and its metadata:

```text
assets/brand/pangram-fox-source.gif
assets/brand/pangram-fox-source.json
```

The metadata file records:

- lowercase SHA-256 of the exact GIF bytes
- original filename and acquisition date
- source owner
- source URL or transfer reference
- written-permission reference
- raster dimensions, frame count, per-frame delay, and cycle duration
- generator version

The generator verifies the hash before decoding frames. A source change
requires a new hash, regenerated frames, and fresh acceptance evidence.

The locked source is the 1772x709 GIF supplied on 2026-08-26. It contains nine
full-canvas frames at 70 ms each for a seamless 630 ms cycle. Its SHA-256 is
`fa806f95e5775e9bc4ffda599a540910edd2042115eae80729308b02d89a542e`.

## Rights gate

The user confirmed Pangram's permission and approved this exact source for:

- use of the fox mark in an unofficial third-party CLI and documentation
- modification into terminal-cell derivatives
- public source and binary redistribution
- GitHub, npm, Homebrew, and Scoop mirrors
- screenshots and recordings
- the relationship between the art and the repository's MIT license

The confirmation does not place the artwork under MIT. The source and generated
art remain outside the repository's MIT grant and carry a separate notice.

## Acceptance evidence

One generated baseline is required, not three design variants. The acceptance
artifact records:

- source and generator hashes
- exact 14-frame 72x16 cycle table, eight generated dissolve frames, and the
  56-entry playback sequence
- palette and fallback assertions
- first-cycle and final-dissolve snapshots
- backdrop-to-canvas and canonical Analyze fade assertions
- reduced-motion and motion-off suppression assertions
- skip-key behavior
- 100x28 minimum and resize behavior
- contrast and legibility review
- terminal restoration result

Automated checks must pass before subjective review. Final user approval is
recorded against the generated artifact set, not against unspecified taste.
