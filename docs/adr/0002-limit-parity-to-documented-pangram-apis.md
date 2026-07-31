# ADR 0002: Limit parity to documented Pangram APIs

Status: accepted
Date: 2026-07-23
Amended: 2026-07-31

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

Product announcements and model cards may establish output semantics, model
limitations, and deprecation dates, but they do not substitute for a
documented request field. A temporary upstream default is not a stable
contract. Pangram CLI must not submit Pangram 4 work until the public API
documents how to select it.

The Pangram SDK v1.0.0 release satisfied the selection condition for text and
bulk on 2026-07-29. The official Mintlify API reference also documents the
Pangram 4 bulk billing rule and request limit: one unit per started 100-word
block per valid item, a minimum of one per item, and at most 1,000 units per
request. Text and bulk are no longer blocked on public request documentation.
Bulk still requires Phase 3 implementation and live conformance.

File analysis and plagiarism remain release-gated until live conformance
resolves their documented response conflicts.

## Consequences

- "Parity" has a stable, reviewable meaning.
- The CLI may intentionally omit web-only account and dashboard features.
- New documented Pangram operations can extend the parity matrix through a
  contract-first change.
- Pangram 4 text submission uses the documented selector without a default
  model fallback.
- Pangram 4 bulk implementation may proceed against the documented selector,
  billing rule, and request limit; public support still requires live
  conformance.
- Public release still needs written Pangram permission even when only public
  endpoints are used.

## Enforcement

- The parity matrix links each shipped workflow to a documented endpoint.
- Protocol tests reject requests to unlisted hosts and paths.
- Every observable protocol change updates the contract artifact first.
- Public release checks written permission and live conformance evidence.
