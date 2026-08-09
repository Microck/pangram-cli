---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Added

Pinned the Phase 4 local-history dependency foundation: an exact,
locked `rusqlite = "=0.39.0"` with `default-features = false` and only the
`bundled` feature. The bundled build compiles vendored SQLite 3.51.3 through
the transitive `libsqlite3-sys 0.37.0` with FTS5, FTS3, RTREE, JSON1, and
default-on foreign key enforcement (`SQLITE_DEFAULT_FOREIGN_KEYS=1`). The
newer `rusqlite 0.40.1`/`libsqlite3-sys 0.38.1` pair was rejected because its
stable `cfg_select!` usage effectively requires Rust 1.95, above the package
MSRV of 1.87; rusqlite 0.39.0's `^0.37.0` sys requirement cannot float to
0.38.x under Cargo's 0.x caret semantics and the lockfile pins 0.37.0, so no
direct sys dependency is carried. The selection excludes the default `cache`
(hashlink) feature, adds no async database, repository, or migration
framework, and carries no public API or behavioral change yet; history
storage remains unimplemented and disabled by default. A focused contract
probe proves the compiled runtime reports SQLite 3.51.3 and `ENABLE_FTS5`,
executes the history contract's FTS5 virtual-table statement, and honors
foreign-key enforcement, grounding the locked architecture choice in
compiled evidence.
