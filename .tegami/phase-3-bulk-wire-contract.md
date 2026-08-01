---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Changed

Pangram CLI's bulk analysis domain and its internal loopback protocol fixture
now lock the officially documented Pangram 4 bulk wire contract. Submission
sends one job-wide `model` set to `pangram-4` with no per-item selector and no
public-dashboard-link field; each valid item is estimated at one billable unit
per started 100-word block with a minimum of one; the job estimate is compared
against the smaller of the caller-supplied ceiling and Pangram's 1,000-unit
request limit before any credential or network work; and the typed
submit/status/items/results responses follow the documented shapes
(page limit 1,000, epoch-second string timestamps, 48-hour terminal metadata
retention, and the documented error matrix).

The bulk and task commands remain planned for Phase 3. No public CLI, MCP, or
README capability is enabled, and public bulk support still requires the
production client plus live conformance. This fragment uses `patch`, matching
the existing contract-and-domain-foundation fragments in this repository.
