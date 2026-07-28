---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Changed

Updated the planned MCP server contract to protocol version 2026-07-28. File
tools now require explicit startup-approved directories, and the server no
longer promises the removed core Tasks lifecycle.
