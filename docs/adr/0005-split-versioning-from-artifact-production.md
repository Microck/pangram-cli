# ADR 0005: Split versioning from artifact production

Status: accepted
Date: 2026-07-23

## Context

Tegami can manage changes, version pull requests, changelogs, and registry
publish locks. The Rust application also needs target-specific archives,
checksums, a signed update manifest, and GitHub Releases. Giving both
systems ownership of tagging or publishing would create race conditions and
ambiguous recovery.

## Decision

Give each release tool one owner role.

Tegami owns:

- change entries
- version selection
- changelog generation
- the version pull request
- the registry publish lock
- npm publication
- retryable registry publish state

The Rust release workflow, using cargo-dist where it earns its place, owns:

- builds and target archives
- checksums and artifact provenance
- the Ed25519-signed update manifest
- the release tag
- the GitHub Release
- Homebrew and Scoop publication jobs

The artifact workflow starts only from the merged version commit and publishes
each target in the order defined by the release plan. It does not recalculate
the version or changelog.

Project agent instructions are maintained by hand. Do not run
`tegami init-agent`, because its generated generic instructions would replace
project-specific guidance and the command is not idempotent.

## Consequences

- One workflow chooses the version and one produces artifacts.
- Release failures can resume by owned stage without competing tags.
- cargo-dist remains an implementation aid, not the source of version truth.
- Release automation must test its handoff between the version commit and
  artifact workflow.

## Enforcement

- CI rejects tag creation outside the artifact workflow.
- The artifact workflow verifies that package versions match the merged
  version commit.
- Registry publication requires the Tegami lock.
- A release is complete only after the signed manifest, hashed artifacts,
  packages, GitHub Release, and smoke tests agree on one version.
