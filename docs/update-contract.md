# Pangram CLI update contract

Status: approved for implementation
Schema major: `"1"`

This file owns the signed direct-update contract. The executable may replace
itself only when its protected installation receipt has `method: direct` and
still identifies both the executable path and executable bytes.

## Release gate and production availability

Production update locations are fixed:

```text
https://github.com/Microck/pangram-cli/releases/latest/download/pangram-update-manifest.json
https://github.com/Microck/pangram-cli/releases/latest/download/pangram-update-manifest.json.sig
```

These planned locations intentionally return 404 until the first authorized
release publishes matching assets.

Alternate locations and public keys exist only in test constructors until a
public release receives the required distribution and signing authorization.
Every private `0.x` production binary embeds an empty update key ring. Its
production updater performs zero DNS, proxy, HTTP, or other network activity.
`pangram update --check`, `pangram update`, and `pangram update --yes` return
the typed `update_unavailable` failure with `retryable: false`. This is a
release gate, not a fallback unsigned updater.

Authorizing production update networking requires one release change that
adds at least one reviewed public key, enables the fixed locations, and proves
the signed release pipeline. Merely publishing a manifest or setting an
environment variable cannot enable it.

## Versions and status

Every version in update state, receipts, manifests, and `UpdateStatus` is a
canonical SemVer 2.0.0 string. It permits valid prerelease and build metadata,
uses no `v` prefix, and forbids leading zeroes where SemVer forbids them.
Comparison uses SemVer precedence. Build metadata does not affect precedence,
so changing build metadata alone is `no_update`.

`update_check` and `update_install` use one closed `UpdateStatus` object. The
field matrix is exact:

| `status` | `current_version` | `available_version` | `manager` | `manager_command` |
| --- | --- | --- | --- | --- |
| `no_update` | required | absent | absent | absent |
| `update_available`, no manager advisory | required | required with greater SemVer precedence | absent | absent |
| `update_available`, manager advisory | required | required with greater SemVer precedence | required | required |
| `updated` | required and equal to the installed target | absent | absent | absent |

The closed `manager` values are `homebrew`, `scoop`, `npm`, `pnpm`, and `bun`.
`manager` and `manager_command` are an all-or-nothing pair. The command is one
sanitized, nonempty shell command for the manager that owns the running
executable. It is display data and is never executed by Pangram. Manager
ownership is carried by the distribution artifact; path guessing does not
create ownership. pnpm and Bun consume the npm package but retain their own
manager identity so the recovery command names the manager the user invoked.

The manager commands are exact:

| `manager` | `manager_command` |
| --- | --- |
| `homebrew` | `brew upgrade Microck/pangram-cli/pangram` |
| `scoop` | `scoop update pangram` |
| `npm` | `npm install --global @microck/pangram-cli@latest` |
| `pnpm` | `pnpm add --global @microck/pangram-cli@latest` |
| `bun` | `bun add --global @microck/pangram-cli@latest` |

Each manager distribution supplies a protected owner marker or launcher
identity bound to that exact distribution. The shared native executable does
not infer ownership from a familiar directory. A missing, conflicting, or
insecure manager identity returns `update_not_owned` rather than guessing.

An update check reports availability independently from mutation ownership.
Absent manager fields mean only that no manager advisory was proven; they do
not prove a direct install. A later install attempt separately requires a
matching protected direct receipt and returns `update_not_owned` when that
receipt is absent or invalid. Proven manager evidence adds the manager pair as
advice, but never grants Pangram mutation authority.

An equal available version produces `no_update`. A lower available version is
rejected as `update_verification_failed`, not reported as `no_update`.
`update_unavailable`, `update_not_owned`, `update_verification_failed`, and
`update_replace_failed` are failure envelopes and are never `UpdateStatus`
values.

## Local state, receipt, and lock

Update-check state is cache data under the platform data directory:

```json
{
  "schema_version": "1",
  "last_checked_at": "2026-07-23T12:00:00Z",
  "etag": "\"example\"",
  "available_version": "1.2.3"
}
```

A direct-install receipt is install-location state:

```json
{
  "schema_version": "1",
  "method": "direct",
  "executable_path": "/home/user/.local/bin/pangram",
  "executable_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "installed_version": "1.2.2",
  "target": "x86_64-unknown-linux-gnu",
  "manifest_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "installed_at": "2026-07-23T12:00:00Z"
}
```

The exact state path is `Paths::data_dir()/update-state.json` and its protected
lock is `Paths::data_dir()/update-state.lock`. For executable path `P`, the
exact receipt path is `P.receipt.json`, the exact pending-finalization path is
`P.update-pending.json`, and the protected mutation lock is `P.update.lock`.
For example, `pangram.exe` uses `pangram.exe.receipt.json`. Install-location
files move only through an explicit installer operation; changing the data
directory never relocates them.

Only `method: direct` is mutable by `pangram update`. Before mutation, the
updater requires the canonicalized running executable path to equal
`executable_path` and the SHA-256 of its exact bytes to equal
`executable_sha256`. A missing, malformed, mismatched, or insecure receipt is
not ownership evidence.

The state lock serializes checks and update-state writes without requiring
write access beside the executable. The mutation lock serializes direct
install, replacement, pending receipt finalization, receipt writes, and
uninstall for `P`. Check acquires only the state lock. Receipt finalization and
uninstall acquire only the mutation lock. An update install that needs both
always acquires the state lock first and the mutation lock second, and releases
them in reverse order. No code path may acquire them in the opposite order.
There is no second receipt or replacement lock.

Failure to create, protect, or acquire a required lock fails before network or
mutation. The implementation may report a bounded busy failure; it MUST NOT
break an apparently live lock. Each lock and protected file requires owner-only
permissions or ACLs. State, receipt, and pending writes use a protected
temporary sibling file, `fsync` where supported, and atomic rename.

On HTTP 200, a successful check replaces `etag`, `last_checked_at`, and
`available_version`. On HTTP 304, it updates `last_checked_at` and preserves
the other fields. A network or verification failure preserves the prior
state. Clock rollback does not suppress a check. When no newer release exists,
`available_version` is removed.

The machine contracts are:

- [`update-state.schema.json`](../contracts/update-state.schema.json)
- [`install-receipt.schema.json`](../contracts/install-receipt.schema.json)

## Untrusted-input limits

Every limit is checked while streaming, before an unbounded allocation:

- manifest response body: at most 1 MiB
- detached-signature response body: at most 16 KiB
- decoded Ed25519 signature: exactly 64 bytes
- `key_id`: 1 through 128 printable ASCII bytes
- stored ETag: at most 1,024 printable ASCII bytes, excluding control bytes
- redirects: at most 5, with HTTPS required on every production hop
- archive bytes: at most 1 GiB and exactly the signed `size_bytes`
- expanded executable: at most 256 MiB and exactly the signed
  `executable_size_bytes`
- XZ dictionary memory: at most 64 MiB, checked before decoder allocation

An oversized or malformed signature, manifest, ETag, archive, or executable is
`update_verification_failed`. A 200 response with an invalid ETag may otherwise
succeed, but the ETag is omitted rather than truncated. Redirects never carry
credentials or authorization headers. Production redirects may change host
only through an HTTPS `Location`; alternate schemes, embedded credentials,
fragments, and more than five hops are rejected.

## Manifest and signature

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
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
    }
  ]
}
```

The versioned URLs in this example are illustrative and intentionally do not
identify an existing release.

Closed values:

- `channel`: `stable`
- `archive_format`: `tar.xz`, `zip`

The exact downloaded bytes of `pangram-update-manifest.json` are signed. The
verifier MUST NOT decode as UTF-8, parse JSON, normalize line endings, or
reserialize before Ed25519 verification. It first reads the bounded detached
signature object, selects an exact `key_id` from the embedded key ring, and
verifies the exact manifest bytes. Only a successful signature permits UTF-8
decoding, JSON parsing, schema validation, target selection, or use of any
manifest URL. The signature value conforms to
[`manifest-signature.schema.json`](../contracts/manifest-signature.schema.json).

The signed artifact size and SHA-256 bind each download. The updater does not
maintain a second per-artifact signature format. Artifact targets are unique;
duplicate targets are rejected even if JSON Schema cannot express uniqueness
by a child field.

The production key ring stays empty until the release authorization above.
After authorization, a manifest may use any currently trusted key. Rotation
ships an overlap release that trusts both old and new keys before the release
service uses only the new key. Removing a key requires a later binary. If all
trusted keys are compromised, recovery is an out-of-band reinstall. There is
no unsigned revocation path.

## Rejection rules

An updater rejects:

- unknown schema major or object fields at a closed integrity boundary
- any non-stable channel
- unsupported target or archive
- invalid or unknown signing key
- a running updater version below `minimum_updater_version`
- a target version lower than the installed version
- size or hash mismatch
- a non-HTTPS production URL or redirect hop
- more than one artifact for the selected target

Manager-owned installations never replace their executable. A valid check may
return `update_available` with the exact manager identity and display command.
An install attempt returns `update_not_owned` with the same recovery command.
An unowned install with no manager evidence can still report availability, but
its install attempt returns `update_not_owned` without inventing advice.

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
`executable_size_bytes`. Validation completes before replacement.

## Confirmation contract

`pangram update --check` never prompts or installs. Bare `pangram update` may
prompt only when stdin, stdout, and stderr are all TTYs and `CI` is unset. The
prompt identifies the current and target version. Declining or interrupting
the confirmation performs no mutation and exits 130.

The private-build empty-key-ring gate runs before confirmation, lock
acquisition, or manager detection, so it returns `update_unavailable` without
prompting or touching update state.

`pangram update --yes` is the only noninteractive installation form. In CI or
when any standard stream is not a TTY, bare `pangram update` fails with
`input_required` and exit 2 before download or mutation. `--yes` accepts no
prompt input and does not require a TTY.

## Direct installer and receipt

The POSIX and PowerShell installers must:

1. fetch the bounded signature and manifest
2. verify the exact manifest bytes with an authorized embedded public key
3. parse the verified manifest and select the exact target
4. verify archive size, SHA-256, layout, and executable size
5. refuse an executable not owned by a matching direct receipt
6. smoke-test the staged candidate
7. write protected pending receipt state and install atomically
8. atomically finalize a receipt containing the installed executable SHA-256

The project does not assume that `openssl`, platform PowerShell, `ssh-keygen`,
or another optional host command supplies a portable Ed25519 verifier. It MUST
NOT replace signature verification with a hash, TOFU key, downloaded verifier,
or an unverified invocation of the candidate binary. A direct installer does
not ship until one implementation proves the same trust root and exact-byte
verification on every supported clean-machine baseline.

The POSIX default is `$HOME/.local/bin/pangram`. The PowerShell default is
`%LOCALAPPDATA%\Programs\Pangram\bin\pangram.exe`. Installers do not edit shell
profiles or system PATH. They print exact PATH instructions when needed.

Uninstall removes the executable, receipt, and finalized pending state only
when all recorded identities still match. It never recursively deletes the
install parent.

## Replacement and receipt finalization

The updater and direct installer download and stage on the executable's
filesystem. Before any replacement they complete signature, manifest, target,
archive, byte-size, hash, permission, ownership, and archive-layout
validation. They then run the staged candidate as `pangram --version` and
require exact success for the signed target version. Any failure through this
candidate smoke test preserves an existing executable and receipt byte for
byte.

On Unix, the updater atomically renames the verified, smoke-tested candidate
into place. On Windows, that candidate enters one narrowly scoped replacement
mode, waits for the parent PID to exit, and replaces only the path and manifest
identity already recorded in protected pending state. The helper accepts no
arbitrary source, destination, command, or manifest URL.

The protected pending record is fully written before replacement and binds the
old and new executable hashes, target version, manifest hash, and receipt path.
After replacement, no download, verification, smoke, rollback, or second
replacement may be retried. The only retryable operation is atomic
finalization of the new receipt from that exact pending record. Finalization
sets `executable_sha256` to the installed candidate hash and then removes the
pending record. A finalization failure reports retryable
`update_replace_failed`; the next install command must finalize it before any
network request or new replacement attempt. Install acquires the state lock,
then `P.update.lock`, and finalizes before releasing either lock. An explicit
check remains state-lock-only and does not inspect or finalize
executable-adjacent pending state; it may report availability normally. Pending
state that does not match the installed bytes fails closed and requires an
explicit reinstall.
