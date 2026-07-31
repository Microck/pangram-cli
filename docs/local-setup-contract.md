# Pangram CLI local-setup contract

Status: approved for implementation
Schema major: `"1"`

This file is the contract owner for local configuration, credential
persistence, and non-billable diagnostics. The observable contract entry point
is [contracts.md](contracts.md); when the two disagree, this file defers to
the entry point.

## Configuration contract

Canonical non-secret TOML:

```toml
config_version = 1

[history]
enabled = false

[tui]
intro = "once"
keymap = "regular"
motion = "full"

[updates]
# Omitted until first-launch choice.
check_on_tui_start = true

[network]
max_requests_per_second = 5
```

Closed values:

- `tui.intro`: `once`, `always`, `off`
- `tui.keymap`: `regular`, `vim`
- `tui.motion`: `full`, `reduced`, `off`

Rules:

- `tui.intro` controls frequency; `tui.motion` controls presentation
- `tui.intro = "once"` records completion in local TUI state without rewriting
  user configuration
- `tui.intro = "off"` and `tui.motion = "off"` both suppress the intro
- unknown keys fail validation
- `config_version` is required
- `network.max_requests_per_second` is greater than 0 and no greater than 5
- `updates.check_on_tui_start` may be omitted before onboarding
- `config get` and `config list` report the effective configuration and
  therefore always agree. A key absent from the file resolves to its
  documented built-in default: `tui.intro` reads as `"once"`, booleans and
  numbers keep their typed JSON representation, and no placeholder or sentinel
  string ever appears. The one exception is `updates.check_on_tui_start`
  before onboarding, which has no built-in default: `config list` omits its
  section and `config get` reports `null`, meaning "no value is configured
  yet". No key ever surfaces a sentinel such as `(unset)` in any projection.
- credentials MUST NOT appear in this file or its generated schema
- no output-format setting
- no public-link setting
- no endpoint setting
- no telemetry setting
- no project profiles

Environment:

| Variable | Meaning |
| --- | --- |
| `PANGRAM_API_KEY` | Ephemeral credential override |
| `PANGRAM_CONFIG` | Explicit config file |
| `PANGRAM_DATA_DIR` | Explicit history and state directory |
| `NO_COLOR` | Disable terminal color |
| `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` | Standard proxy behavior |
| `CI` | Disable interactive and automatic-update behavior |

General precedence is flags, environment, explicit config, default config,
built-ins. Credentials use environment then stored key.

The stored key lives in a dedicated `credentials.toml` in the default platform
configuration directory. `PANGRAM_CONFIG` never relocates it. The file contains
only `credentials_version = 1` and `api_key`. It requires mode `0600` on Unix or
an owner-only ACL on Windows. Creation and every read fail closed when the
restriction cannot be established.

### Intro startup contract

An intro-eligible launch has an interactive TTY, does not run under `CI`, has
`TERM` other than `dumb`, and has reached the minimum 80x24 terminal size.
Re-entering the alternate screen within the same process is not a new launch.
All three standard streams must be TTYs.

The resolved behavior is:

| `tui.intro` | `tui.motion` | Behavior |
| --- | --- | --- |
| `off` | any | Open Analyze immediately |
| `once` | `full` | Play once, then open Analyze |
| `once` | `reduced` | Show the resolved mark in the first interactive Analyze frame once |
| `once` | `off` | Open Analyze immediately without consuming the one-time state |
| `always` | `full` | Play on every eligible launch |
| `always` | `reduced` | Show the resolved mark in the first interactive Analyze frame |
| `always` | `off` | Open Analyze immediately |

For `once`, completing the full intro, skipping it, or rendering the reduced
first frame atomically records `intro_seen = true` in local TUI state under
`PANGRAM_DATA_DIR`. A suppressed or ineligible launch does not consume the
one-time state. The marker is state, not configuration, and never changes the
value returned by `pangram config get tui.intro`.

The marker file is `PANGRAM_DATA_DIR/tui-state.json`:

```json
{
  "schema_version": "1",
  "intro_seen": true
}
```

The TUI writes it through a temporary sibling file and atomic rename. A
missing file means unseen. An unreadable, invalid, or unwritable state file
produces a non-blocking diagnostic; invalid or unreadable state is treated as
unseen, and the failure MUST NOT prevent Analyze from opening. Its machine
contract is [tui-state.schema.json](../contracts/tui-state.schema.json).

## Doctor diagnostics contract

`pangram doctor` is a Phase 1 non-billable diagnostic command. It performs
no Pangram network request, no DNS resolution, and no credential validation
against Pangram. It never creates or mutates the data directory, the
configuration file, or the credential store.

The `data` payload is a typed `DoctorStatus` object with an ordered `checks`
array. Phase 1 uses the following closed, ordered check names:

1. `configuration`
2. `credentials`
3. `data_directory`
4. `runtime`

Check semantics and closed statuses:

- `configuration`: `pass` when the effective strict configuration loads
  successfully; `fail` with a sanitized message on any configuration error.
- `credentials`: `pass` when a valid credential source (environment or
  stored) is resolved; `warn` when no credential is configured, with guidance
  pointing to `https://www.pangram.com/apikey`; `fail` for unreadable or
  insecure stored credentials. The check never emits the API key, its masked
  suffix, or any credential material.
- `data_directory`: `pass` when the resolved data-directory path exists, is
  a directory, and is readable; `warn` when the path is absent because later
  features create it lazily; `fail` when the path exists but is not a
  directory or is unreadable. The check does not create the directory.
- `runtime`: always `pass`; its message safely names the package version,
  target OS, target architecture, and whether the process is running under
  CI. The message contains no environment dump.

The `doctor` command MUST return the complete `checks` array even when one
or more checks fail. Diagnostics errors are reserved for impossible
output-construction failures only, not for unhealthy local state.

Exit behavior is driven by check health, not by the selected format:

- every check `pass` or `warn`: the process exits 0
- any check `fail`: the process exits 7 (the canonical
  local-configuration/history/update-state code) after emitting the complete
  report

The report keeps the canonical success envelope (`data`, never `error`) even
at exit 7, because the command itself succeeded and the payload is a typed
status object. The pretty projection prints the same complete ordered checks
and follows the identical exit rule. An output-construction or stdout write
failure remains a general failure (exit 1) rather than being overwritten by
the health-derived exit code.
