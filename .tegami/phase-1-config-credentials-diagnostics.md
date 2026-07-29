---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Added

Added the configuration, credential, and local diagnostics layer: strict TOML
configuration with documented precedence, dedicated atomic `credentials.toml`
persistence with Unix `0600` and owner-only protected Windows ACL enforcement,
`auth`, `config`, and non-billable `doctor` commands, and a Windows CI gate
that runs the credential ACL integration tests on real Win32 security APIs.

## Security

Credential persistence fails closed when restrictive file permissions or an
exact single-ACE owner-only DACL cannot be established, and credential
material is never rendered into errors, diagnostics, or logs.

Review remediation hardened the same guarantees further: `credentials.toml`
existence is probed with an error-preserving metadata call so an unsearchable
credential store reports `fail` instead of being mistaken for an absent key;
the `doctor` data-directory check now proves readability with a non-mutating
directory open instead of trusting `metadata.is_dir()` alone, and every
diagnostic message and configuration error is stripped of ASCII control
characters (newline, carriage return, tab, `ESC`, and `DEL`) so a
user-controlled path cannot inject terminal sequences or forge extra check
lines at the projection boundary. Bare `pangram auth` now honors the
documented `CI`-disables-interactivity rule and falls back to the typed
`auth status` report even when every stream is a TTY, and `auth logout`
without `--yes` makes the same CI determination before it would otherwise
block on an interactive confirmation. Credential removal is now idempotent on
Windows too: an absent `credentials.toml` is matched as success before ACL
verification, so `auth logout --yes` no longer surfaces a spurious
restriction error for a path that is already gone.

Secret-material hardening tightened further under review: temporary files are
created with mode `0600` atomically at `open(2)` on Unix rather than narrowed
after the fact, diagnostic messages now strip every Unicode control character
(including the C1 CSI range) instead of only ASCII controls, an 8-character
or shorter stored key is never reproduced verbatim by `auth status` (it maps
to a constant masked marker), the ephemeral `PANGRAM_API_KEY` override is
held in a zeroizing buffer so merged/cloned copies do not linger in freed
heap memory, a no-subcommand invocation carrying only global flags displays
help with a successful exit, and the Win32 ACL/token buffers are allocated
with struct alignment (DWORD/pointer) rather than byte alignment. On Windows
the owner-only DACL is applied to the credential temp file through
`SetSecurityInfo` on the open handle rather than by re-resolving its name, so
the descriptor is tightened on the object before any content is written and
the name-based lookup/lossy-path translation is removed from the
creation-time path; the short-key status mask now uses ASCII-only characters.
