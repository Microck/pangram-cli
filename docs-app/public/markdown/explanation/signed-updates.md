# How signed releases and updates work

The updater verifies an Ed25519 signature over the exact downloaded manifest
bytes before it parses JSON. The signed manifest binds target, version,
archive size, executable size, and SHA-256. Only an exact direct-install
receipt grants mutation authority. Manager-owned binaries receive manager
instructions and are never adopted by the self-updater.
