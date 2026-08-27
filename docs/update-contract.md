# Pangram CLI update contract

Status: approved for implementation
Schema major: `"1"`

This file is the contract owner for signed direct updates. The executable may
replace itself only when its installation receipt has `method: direct`.

Production locations are fixed:

```text
https://github.com/Microck/pangram-cli/releases/latest/download/pangram-update-manifest.json
https://github.com/Microck/pangram-cli/releases/latest/download/pangram-update-manifest.json.sig
```

Alternate locations exist only in test constructors.

`0.x` binaries expose the three `pangram update` forms so scripts can depend on
the final command grammar, but they perform no release network request, prompt,
state read, or mutation. This remains true for an authorized public `0.x`
release. `pangram update --check`,
bare `pangram update`, and `pangram update --yes` all return the canonical
`update_unavailable` failure before any other updater work. This major-version
guard remains the outermost updater policy until `1.0.0`.

## Local state and receipt

Update-check state is data, not user configuration:

```json
{
  "schema_version": "1",
  "last_checked_at": "2026-07-23T12:00:00Z",
  "etag": "\"example\"",
  "available_version": "1.2.3"
}
```

Direct-install receipt:

```json
{
  "schema_version": "1",
  "method": "direct",
  "executable_path": "/home/user/.local/bin/pangram",
  "installed_version": "1.2.2",
  "target": "x86_64-unknown-linux-gnu",
  "manifest_sha256": "...",
  "installed_at": "2026-07-23T12:00:00Z"
}
```

Only `method: direct` is mutable by `pangram update`.

The machine contracts are:

- [`update-state.schema.json`](../contracts/update-state.schema.json)
- [`install-receipt.schema.json`](../contracts/install-receipt.schema.json)

State writes use a temporary sibling file, `fsync` where supported, and atomic
rename. On HTTP 200, a successful check replaces `etag`, `last_checked_at`, and
`available_version`. On HTTP 304, it updates `last_checked_at` and preserves the
other fields. A network or verification failure preserves the prior state.
Clock rollback does not suppress a check. When no newer release exists,
`available_version` is removed.

The state and receipt live under the platform data directory and require
owner-only file permissions or ACLs. A receipt that cannot be protected or
validated is not update ownership evidence.

## Manifest

Canonical unsigned manifest value:

```json
{
  "schema_version": "1",
  "channel": "stable",
  "version": "1.2.3",
  "published_at": "2026-07-23T12:00:00Z",
  "notes_url": "https://github.com/Microck/pangram-cli/releases/tag/v1.2.3",
  "minimum_updater_version": "1.0.0",
  "artifacts": [
    {
      "target": "x86_64-unknown-linux-gnu",
      "archive_format": "tar.xz",
      "url": "https://github.com/Microck/pangram-cli/releases/download/v1.2.3/pangram-v1.2.3-x86_64-unknown-linux-gnu.tar.xz",
      "size_bytes": 1234567,
      "executable_size_bytes": 3456789,
      "sha256": "..."
    }
  ]
}
```

Closed values:

- `channel`: `stable`
- `archive_format`: `tar.xz`, `zip`
The exact downloaded UTF-8 bytes of `pangram-update-manifest.json` are signed.
The verifier MUST NOT parse and reserialize before verification. The detached
signature is a JSON value conforming to
[`manifest-signature.schema.json`](../contracts/manifest-signature.schema.json).
After signature verification, the updater parses and validates the manifest.
The signed artifact size and SHA-256 bind each download; the updater does not
maintain a second per-artifact Ed25519 signature format.

Artifact targets are unique. The implementation rejects a manifest containing
duplicate targets even if the JSON Schema validator cannot express that
property.

## Rejection rules

An updater rejects:

- unknown schema major
- any non-stable channel
- unsupported target or archive
- invalid or unknown signing key
- a running updater version below `minimum_updater_version`
- a target version lower than the installed version
- size or hash mismatch
- a non-HTTPS production artifact URL

An equal target and installed version is `no_update`, not an error. Downgrades
are not supported by `pangram update`.

The binary embeds a key ring of key IDs and public keys. A manifest may be
signed by any currently trusted key. Rotation ships an overlap release that
trusts both old and new keys before the release service begins using only the
new key. Removing a key requires a later binary. If every trusted key is
compromised, recovery is an out-of-band reinstall; the updater does not accept
an unsigned revocation document.

Manager-owned installations never replace their executable. They return the
detected manager and its update command. npm ownership is recognized only for
native executables below the five shipped platform packages:
`@microck/pangram-cli-darwin-arm64`, `@microck/pangram-cli-darwin-x64`,
`@microck/pangram-cli-linux-arm64`, `@microck/pangram-cli-linux-x64`, and
`@microck/pangram-cli-win32-x64`. The advice for each is
`npm update --global @microck/pangram-cli`.

## Archive contract

Release archives may contain only:

- one root executable named `pangram` or `pangram.exe`
- `README.md`
- `LICENSE`
- files below `completions/`
- files below `man/`

The updater extracts only the executable. It rejects absolute paths, `..`,
symlinks, hardlinks, devices, duplicate executable entries, unexpected root
entries, and an executable whose expanded size differs from
`executable_size_bytes`. Validation occurs before replacement.

## Direct installer and receipt

The POSIX and PowerShell installers:

1. fetch and verify the detached manifest signature
2. select the exact target and verify archive size and SHA-256
3. validate the archive contract
4. refuse to overwrite an executable not owned by a matching direct receipt
5. install atomically
6. run `pangram --version`
7. atomically write the direct-install receipt only after the smoke test passes

Each versioned installer script embeds the selected archive URL, byte size,
and SHA-256 value generated from the signed manifest. The script verifies that
identity before it executes the archive candidate. The candidate then verifies
the downloaded manifest's detached Ed25519 signature with the production key
ring, selects its exact compile target, validates the complete archive contract,
and proves its own executable bytes equal the executable extracted from that
archive. Destination mutation begins only after both layers succeed. The hidden
candidate mode accepts local manifest, signature, archive, and destination
paths; it is not part of the public Clap grammar and never accepts an alternate
key or network endpoint.

The installer fetches the manifest, detached signature, and archive from the
same immutable `releases/download/vVERSION` location. The mutable `latest`
locations above belong only to an installed updater checking for a newer
release. A versioned installer never combines them with its pinned candidate.
The POSIX installer selects GNU Linux archives only after
`getconf GNU_LIBC_VERSION` proves the host uses glibc. It rejects musl and an
unknown libc before downloading release files.

Candidate smoke tests retry only transient operating-system spawn contention,
with at most five attempts separated by 25 milliseconds. A process that starts
but exits unsuccessfully or reports the wrong version fails without a retry.

The release workflow renders installers only after signing the manifest. It
runs each native candidate against those exact signed files before any release
or registry publication. Installer templates and rendered scripts contain no
runtime endpoint override. Tests may call the same local candidate mode with
fixture keys through Rust test constructors, but production binaries trust only
the embedded production key ring.

The POSIX default is `$HOME/.local/bin/pangram`. The PowerShell default is
`%LOCALAPPDATA%\Programs\Pangram\bin\pangram.exe`. Installers do not edit shell
profiles or system PATH. They print exact PATH instructions when needed.

Uninstall removes the executable and receipt only when both still match the
receipt. It never recursively deletes the install parent.

## Replacement and receipt update

The updater downloads to the executable's filesystem, validates before
mutation, and preserves the running executable on every failure.

On Unix, it atomically renames the verified executable into place. On Windows,
an installed binary replacing its own running executable launches the verified
new executable in a narrowly scoped replacement mode. That process waits for
the parent PID to exit, replaces the recorded path, and runs the version smoke
test. The replacement mode accepts only the exact current path and manifest
identity already recorded in the receipt.

A versioned installer runs from the extracted archive candidate, not from the
installed destination. Its candidate mode therefore replaces the distinct
destination synchronously on Windows, runs the installed-path smoke test, and
publishes the new receipt before it returns. It does not start the deferred
self-replacement mode or depend on the short-lived archive process remaining
open long enough for a child process to acquire its PID.

The new receipt is written only after replacement and the version smoke test
succeed. A failed receipt write reports `update_replace_failed` and preserves
enough verified state to retry receipt finalization without downloading or
replacing again.
