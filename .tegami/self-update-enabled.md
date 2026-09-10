---
packages:
  "cargo:microck-pangram-cli": major
  "npm:@microck/pangram-cli": major
---

## Added

Signed self-update is active. `pangram update --check` reports whether a newer signed release exists, bare `pangram update` asks before installing on a terminal, and `pangram update --yes` installs without asking. The MCP `check_update` tool performs the same read-only check.

## Changed

Update availability now depends on how the executable was installed rather than on the version. A Homebrew, Scoop, or npm installation reports that manager's upgrade command instead of replacing itself, and a copy with no direct-install receipt refuses to replace itself.
