# ADR 0003: Share one analysis module across adapters

Status: accepted
Date: 2026-07-23

## Context

The CLI, TUI, and MCP server need the same operations, progress semantics,
errors, retries, billing safeguards, and normalized results. Implementing an
endpoint wrapper in each adapter would multiply protocol knowledge and allow
observable behavior to drift.

A generic transport trait would also add a shallow interface before a second
production transport exists. Tests can exercise the real HTTP implementation
against a loopback server without module mocks.

## Decision

Place Pangram orchestration behind one deep analysis module.

The module owns:

- request construction and authentication
- billable submission policy
- polling and retry policy
- upstream response normalization
- progress events
- partial-success preservation
- local cancellation of observation

CLI, TUI, and MCP are thin adapters at the analysis seam. They resolve caller
input, invoke typed operations, and project the returned events and values.

Begin with a concrete `Analyzer` whose interface is `start`, `snapshot`,
`observe`, and `bulk_results`. Typed request and reference enums carry the
workflow differences. Do not introduce a generic HTTP port or one public
method per raw endpoint. Add an internal seam only when two real adapters or a
proven test need justify it.

## Consequences

- Protocol fixes have one canonical home.
- All adapters expose the same result and error model.
- Integration tests use a real loopback server and survive internal refactors.
- The analysis module carries substantial implementation depth, so its
  interface must remain small and cohesive.

## Enforcement

- Only the analysis module may call Pangram endpoints.
- Adapter tests assert observable behavior through the analysis interface.
- Static architecture checks reject Pangram HTTP paths outside the module.
- New adapter features must map to an existing typed operation or deepen the
  shared interface through a contract-first change.
