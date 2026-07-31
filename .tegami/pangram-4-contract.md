---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Changed

Pangram CLI now targets Pangram 4 as its only text model. Detection results
include Pangram 4 humanizer evidence, use its three document classifications,
and estimate text billing in started 100-word units.

Text requests use Pangram's documented `pangram-4` selector. The documented
bulk contract uses the same job-wide selector, charges each valid item in
started 100-word units, and caps a request at 1,000 units. Bulk commands remain
planned until Phase 3 implementation and live conformance.
