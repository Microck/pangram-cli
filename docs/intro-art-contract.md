# Pangram CLI intro art contract

Status: blocked pending source geometry and written rights

This contract owns the source, provenance, license boundary, and acceptance
evidence for the generated terminal fox frames. Intro behavior and timing remain
owned by the product contract and ADR 0006.

## Required source

Before frame generation begins, the private development tree must contain:

```text
assets/brand/pangram-fox-source.svg
assets/brand/pangram-fox-source.json
```

The metadata file records:

- lowercase SHA-256 of the exact SVG bytes
- original filename and acquisition date
- source owner
- source URL or transfer reference
- written-permission reference
- view box and geometry dimensions
- generator version

The generator verifies the hash before reading geometry. A geometry change
requires a new hash, regenerated frames, and fresh acceptance evidence.

The previously supplied temporary Litterbox archive is unavailable and returned
HTTP 404 on 2026-07-27. It is not a valid source reference.

## Rights gate

Written Pangram permission must cover:

- use of the fox mark in an unofficial third-party CLI and documentation
- modification into terminal-cell derivatives
- public source and binary redistribution
- GitHub, npm, Homebrew, and Scoop mirrors
- screenshots and recordings
- the relationship between the art and the repository's MIT license

Unless the permission explicitly places the artwork under MIT, the source and
generated art remain outside the MIT grant and receive a separate notice.

## Acceptance evidence

One generated baseline is required, not three design variants. The acceptance
artifact records:

- source and generator hashes
- exact 56-frame full and compact tables
- palette and fallback assertions
- phase-boundary snapshots
- reduced-motion and motion-off snapshots
- skip-key behavior
- 80x24 minimum and resize behavior
- contrast and legibility review
- terminal restoration result

Automated checks must pass before subjective review. Final user approval is
recorded against the generated artifact set, not against unspecified taste.
