# ADR 0002: Limit parity to documented Pangram APIs

Status: accepted
Date: 2026-07-23

## Context

The product goal is feature parity with useful Pangram web workflows. The web
application can use private routes, browser sessions, and behavior that
Pangram has not committed to third-party clients.

Depending on those internals would create a brittle client and may violate
Pangram's terms. The documented API currently covers text AI detection, bulk
AI detection, file AI detection, task status, and plagiarism checks. Some
documented response examples conflict, so even the public surface needs live
conformance before every workflow can ship.

## Decision

Define v1 parity as all useful workflows that can be built from Pangram's
documented REST API.

The project must not:

- scrape the Pangram web application
- use private dashboard routes
- authenticate with browser session cookies
- claim parity for a web feature without a supported public operation

If the web application exposes a useful workflow that the public API cannot
support, record it as an upstream gap. Do not add a compatibility path around
the missing contract.

File analysis and plagiarism remain release-gated until live conformance
resolves their documented response conflicts.

## Consequences

- "Parity" has a stable, reviewable meaning.
- The CLI may intentionally omit web-only account and dashboard features.
- New documented Pangram operations can extend the parity matrix through a
  contract-first change.
- Public release still needs written Pangram permission even when only public
  endpoints are used.

## Enforcement

- The parity matrix links each shipped workflow to a documented endpoint.
- Protocol tests reject requests to unlisted hosts and paths.
- Every observable protocol change updates the contract artifact first.
- Public release checks written permission and live conformance evidence.
