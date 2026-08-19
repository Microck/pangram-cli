# History storage

History uses one SQLite database beneath the platform data directory. It is
off by default and stores plaintext content only after explicit enablement or
`--save`. The database, WAL, shared-memory sidecar, and parent directory use
owner-only permissions or ACLs. Schema drift and unsafe sidecars fail closed.
