# Pangram 4.1: verified changes and client scope

Checked 2026-10-19. This is an implementation boundary, not a claim of complete
live API conformance or an independently verified accuracy benchmark.

## Announced changes

The [release post](https://x.com/max_spero_/status/2112249405684191350) announces
recalibrated confidence and a dashboard estimated AI-percentage range for
less-certain documents. The [short-scan follow-up](https://x.com/max_spero_/status/2112249407156388173)
adds a lower-certainty notice below 150 words. The [accuracy follow-up](https://x.com/max_spero_/status/2112249408888660063)
reports improvements to previously reported false positives. These are distinct
changes; the first screenshot alone was not the complete announcement.

## Implemented here

Pretty, Markdown and TUI AI results display a shared short-text caution based on
the returned segment word counts. No numerical range is fabricated. Existing
classification, fractions and per-segment confidence are preserved, as are
machine-readable output schemas. The model's accuracy changes are server-side;
the CLI does not attempt to reproduce or recalibrate the detector locally.

## Public API evidence and remaining gap

The [AI Detection API](https://docs.pangram.com/api-reference/ai-detection) still
shows the 4.0 result contract, including categorical segment confidence. The
[Models API](https://docs.pangram.com/api-reference/models) documents an ordered,
account-specific selector catalog; a model product name is not proof of a new
request selector. The public SDK at
[e8d737a](https://github.com/pangramlabs/pangram-sdk/commit/e8d737abf513c20808d3665025b76caa6ccfbdb9)
and public documentation at
[19c9200](https://github.com/pangramlabs/docs/commit/19c920036efe8344d75eb8dabba77083fda9eaf9)
do not yet define 4.1 range fields or a changed response-version string.

Accordingly, this patch does not invent a `pangram-4.1` selector, weaken the
version/shape guard, or claim that a dashboard range is available through the
public API. An actual 4.1 wire contract remains to be confirmed before those
changes can ship. No dashboard scraping, browser-session authentication,
provider credits, or public result links are used for verification.
