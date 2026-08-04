//! The exact schema-v1 catalog probe (`docs/history-contract.md` exact v1
//! catalog surface), split out of `store.rs` so both stay under the
//! source-size hygiene thresholds.
//!
//! The probe is the executor of the store's fail-closed contract: an
//! existing `user_version = 1` database opens only when its stored structure
//! matches the contracted v1 surface exactly: the ordered columns with
//! exact declared types/nullability/defaults, the primary keys, the uniqueness
//! surface by origin and ordered columns, the named indexes with their
//! ordered columns and direction, the exact foreign-key set with its
//! actions, and the FTS5 virtual table with its columns and `unicode61`
//! tokenizer. Any deviation, or a probe the connection cannot answer,
//! surfaces one `false`, and the store never repairs, migrates, or rewrites
//! an incompatible body in place.

use rusqlite::Connection;

use super::{HistoryError, HistoryErrorCode, store::SCHEMA_V1};

#[derive(Debug, PartialEq, Eq)]
struct CatalogEntry {
    kind: String,
    name: String,
    table: String,
    sql: Option<Vec<SqlToken>>,
}

#[derive(Debug, PartialEq, Eq)]
enum SqlToken {
    Keyword(String),
    Identifier(String),
    StringLiteral(String),
    Symbol(char),
}

/// One expected column of a v1 base table: its case-sensitive name, its
/// exact declared type, its `NOT NULL` flag, and its
/// default expression exactly as `sqlite_master` records it (`None` for no
/// default), and its `PRAGMA table_xinfo` hidden flag. The v1 base tables
/// declare no defaults or hidden/generated columns.
struct ColumnSpec {
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    default: Option<&'static str>,
    hidden: i64,
}

const fn column(name: &'static str, declared_type: &'static str, not_null: bool) -> ColumnSpec {
    ColumnSpec {
        name,
        declared_type,
        not_null,
        default: None,
        hidden: 0,
    }
}

/// One expected foreign-key rule: `(from column, target table, target
/// column, on_update, on_delete, match)`.
type ForeignKeySpec = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const BULK_COLLECTIONS_COLUMNS: &[ColumnSpec] = &[
    column("id", "TEXT", false),
    column("upstream_bulk_id", "TEXT", false),
    column("status", "TEXT", true),
    column("submission_outcome", "TEXT", true),
    column("total_items", "INTEGER", true),
    column("accepted", "INTEGER", true),
    column("succeeded", "INTEGER", true),
    column("failed", "INTEGER", true),
    column("estimated_billable_units", "INTEGER", true),
    column("created_at", "TEXT", true),
    column("updated_at", "TEXT", true),
    column("completed_at", "TEXT", false),
];

const ANALYSES_COLUMNS: &[ColumnSpec] = &[
    column("id", "TEXT", false),
    column("bulk_id", "TEXT", false),
    column("bulk_index", "INTEGER", false),
    column("caller_id", "TEXT", false),
    column("status", "TEXT", true),
    column("submission_outcome", "TEXT", true),
    column("save_state", "TEXT", true),
    column("input_type", "TEXT", true),
    column("input_sha256", "TEXT", true),
    column("display_name", "TEXT", false),
    column("input_json", "TEXT", true),
    column("result_json", "TEXT", false),
    column("error_json", "TEXT", false),
    column("upstream_version", "TEXT", false),
    column("retry_of", "TEXT", false),
    column("rerun_of", "TEXT", false),
    column("created_at", "TEXT", true),
    column("updated_at", "TEXT", true),
    column("completed_at", "TEXT", false),
];

const UPSTREAM_TASKS_COLUMNS: &[ColumnSpec] = &[
    column("analysis_id", "TEXT", true),
    column("check_kind", "TEXT", true),
    column("upstream_task_id", "TEXT", true),
    column("last_stage", "TEXT", false),
    column("observed_at", "TEXT", true),
];

fn probe_error() -> HistoryError {
    HistoryError::from_sqlite(
        HistoryErrorCode::HistoryUnavailable,
        "schema structure probe",
    )
}

/// The whole exact-schema-v1 structural probe over one borrowed connection.
/// `Ok(true)` opens; `Ok(false)` fails closed with the incompatible-v1
/// `history_corrupt` error the store wraps; `Err` means the catalog itself
/// could not be read (storage unavailable, not corruption).
pub(super) fn schema_structure_ok(connection: &Connection) -> Result<bool, HistoryError> {
    if !canonical_catalog_exact(connection)? {
        return Ok(false);
    }

    // Required table/index names with their expected `sqlite_master`
    // kinds. FTS5 backing tables (`analysis_search_*`) are owned by
    // SQLite; the contract surface is the virtual table itself.
    for (name, kind) in [
        ("bulk_collections", "table"),
        ("analyses", "table"),
        ("upstream_tasks", "table"),
        ("analysis_search", "table"),
        ("analyses_status_created", "index"),
        ("analyses_bulk_index", "index"),
    ] {
        let present: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1 AND type = ?2",
                rusqlite::params![name, kind],
                |row| row.get(0),
            )
            .map_err(|_| probe_error())?;
        if present == 0 {
            return Ok(false);
        }
    }
    // No uncontracted application table may hide beside the required
    // surface. FTS5 owns five shadow tables for `analysis_search`; those
    // are part of that one virtual table's implementation rather than
    // additional application tables.
    let mut tables = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(|_| probe_error())?;
    let table_names = tables
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| probe_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| probe_error())?;
    const EXPECTED_TABLES: [&str; 9] = [
        "analyses",
        "analysis_search",
        "analysis_search_config",
        "analysis_search_content",
        "analysis_search_data",
        "analysis_search_docsize",
        "analysis_search_idx",
        "bulk_collections",
        "upstream_tasks",
    ];
    if table_names != EXPECTED_TABLES {
        return Ok(false);
    }

    // The exact ordered column surface of each base table (the FTS5
    // virtual table is proven separately below): position, name, declared
    // type, nullability, and default are all compared.
    if !columns_exact(connection, "bulk_collections", BULK_COLLECTIONS_COLUMNS)?
        || !columns_exact(connection, "analyses", ANALYSES_COLUMNS)?
        || !columns_exact(connection, "upstream_tasks", UPSTREAM_TASKS_COLUMNS)?
    {
        return Ok(false);
    }

    // The exact ordered primary-key column sets per table.
    if !primary_key_exact(connection, "bulk_collections", &[("id", false)])?
        || !primary_key_exact(connection, "analyses", &[("id", false)])?
        || !primary_key_exact(
            connection,
            "upstream_tasks",
            &[("analysis_id", false), ("check_kind", false)],
        )?
    {
        return Ok(false);
    }

    // The exact uniqueness surface: aside from the `pk` primary keys, each
    // table owns exactly the contracted `u`-origin indexes, no more and no
    // less, and each one covers its exact ordered columns. A
    // `CREATE UNIQUE INDEX` would silently masquerade as a table
    // uniqueness rule here, so the named `c`-origin indexes are proven
    // non-unique below and no `u`-origin entry beyond the contracted set
    // may exist.
    if !unique_indexes_exact(connection, "bulk_collections", &[&["upstream_bulk_id"][..]])?
        || !unique_indexes_exact(connection, "analyses", &[&["bulk_id", "bulk_index"][..]])?
        || !unique_indexes_exact(
            connection,
            "upstream_tasks",
            &[&["check_kind", "upstream_task_id"][..]],
        )?
    {
        return Ok(false);
    }

    // The exact named indexes on `analyses`, created by the schema body
    // (origin `c`): `analyses_status_created` over `(status, created_at)`
    // with `created_at` descending, and `analyses_bulk_index` over
    // `(bulk_id, bulk_index)`, both non-unique and both present exactly
    // once.
    if !named_index_exact(
        connection,
        "analyses",
        "analyses_status_created",
        &[("status", false), ("created_at", true)],
    )? || !named_index_exact(
        connection,
        "analyses",
        "analyses_bulk_index",
        &[("bulk_id", false), ("bulk_index", false)],
    )? {
        return Ok(false);
    }
    // And no uncontracted explicitly named index exists anywhere in the
    // catalog. SQLite autoindexes also have `sqlite_master` rows, but their
    // `sql` is NULL; schema-created named indexes carry their CREATE text.
    let named_indexes: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'index' AND sql IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|_| probe_error())?;
    if named_indexes != 2 {
        return Ok(false);
    }

    // The exact foreign-key surface of each table: the full set of
    // `(from, to-table, to-column, on_update, on_delete, match)` rules,
    // with no entry missing and no extra entry present.
    if !foreign_keys_exact(connection, "bulk_collections", &[])?
        || !foreign_keys_exact(
            connection,
            "analyses",
            &[
                (
                    "bulk_id",
                    "bulk_collections",
                    "id",
                    "NO ACTION",
                    "NO ACTION",
                    "NONE",
                ),
                (
                    "retry_of",
                    "analyses",
                    "id",
                    "NO ACTION",
                    "NO ACTION",
                    "NONE",
                ),
                (
                    "rerun_of",
                    "analyses",
                    "id",
                    "NO ACTION",
                    "NO ACTION",
                    "NONE",
                ),
            ],
        )?
        || !foreign_keys_exact(
            connection,
            "upstream_tasks",
            &[(
                "analysis_id",
                "analyses",
                "id",
                "NO ACTION",
                "CASCADE",
                "NONE",
            )],
        )?
    {
        return Ok(false);
    }

    // `analysis_search` must be the exact FTS5 virtual table: created
    // `USING fts5` with the `unicode61` tokenizer, with its catalog
    // columns in the exact contracted order.
    fts5_exact(connection)
}

/// Compares the complete real catalog with a reference catalog generated by
/// executing the compiled schema-v1 body through the same bundled SQLite
/// engine. This catches declaration semantics SQLite's PRAGMAs intentionally
/// omit (for example `MATCH`, deferral, constraint conflict policies, and
/// FTS5 options) without maintaining a second handwritten schema model.
fn canonical_catalog_exact(connection: &Connection) -> Result<bool, HistoryError> {
    let expected = Connection::open_in_memory().map_err(|_| probe_error())?;
    expected
        .execute_batch(SCHEMA_V1)
        .map_err(|_| probe_error())?;
    Ok(read_catalog(connection)? == read_catalog(&expected)?)
}

fn read_catalog(connection: &Connection) -> Result<Vec<CatalogEntry>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT type, name, tbl_name, sql FROM sqlite_master \
             ORDER BY type, name, tbl_name",
        )
        .map_err(|_| probe_error())?;
    statement
        .query_map([], |row| {
            let sql = row
                .get::<_, Option<String>>(3)?
                .map(|ddl| normalize_sql(&ddl))
                .transpose()
                .map_err(|message| {
                    rusqlite::Error::FromSqlConversionFailure(
                        ddl_column_index(),
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            message,
                        )),
                    )
                })?;
            Ok(CatalogEntry {
                kind: row.get(0)?,
                name: row.get(1)?,
                table: row.get(2)?,
                sql,
            })
        })
        .map_err(|_| probe_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| probe_error())
}

const fn ddl_column_index() -> usize {
    3
}

/// Tokenizes catalog DDL while erasing only spellings the contract declares
/// harmless: keyword case, whitespace/comments, optional trailing semicolons,
/// and identifier quoting. Every semantic token remains in the comparison.
fn normalize_sql(sql: &str) -> Result<Vec<SqlToken>, &'static str> {
    let chars = sql.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            character if character.is_whitespace() => index += 1,
            '-' if chars.get(index + 1) == Some(&'-') => {
                index += 2;
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
            }
            '/' if chars.get(index + 1) == Some(&'*') => {
                index += 2;
                loop {
                    if index + 1 >= chars.len() {
                        return Err("unterminated SQL comment");
                    }
                    if chars[index] == '*' && chars[index + 1] == '/' {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            '\'' => {
                let (literal, next) = quoted_token(&chars, index, '\'', '\'')?;
                tokens.push(SqlToken::StringLiteral(literal));
                index = next;
            }
            '"' => {
                let (identifier, next) = quoted_token(&chars, index, '"', '"')?;
                tokens.push(SqlToken::Identifier(identifier));
                index = next;
            }
            '`' => {
                let (identifier, next) = quoted_token(&chars, index, '`', '`')?;
                tokens.push(SqlToken::Identifier(identifier));
                index = next;
            }
            '[' => {
                let (identifier, next) = quoted_token(&chars, index, '[', ']')?;
                tokens.push(SqlToken::Identifier(identifier));
                index = next;
            }
            character if is_word_character(character) => {
                let start = index;
                index += 1;
                while index < chars.len() && is_word_character(chars[index]) {
                    index += 1;
                }
                let word = chars[start..index].iter().collect::<String>();
                let lower = word.to_ascii_lowercase();
                if is_sql_keyword(&lower) {
                    tokens.push(SqlToken::Keyword(lower));
                } else {
                    tokens.push(SqlToken::Identifier(word));
                }
            }
            ';' => {
                tokens.push(SqlToken::Symbol(';'));
                index += 1;
            }
            symbol => {
                tokens.push(SqlToken::Symbol(symbol));
                index += 1;
            }
        }
    }
    if matches!(tokens.last(), Some(SqlToken::Symbol(';'))) {
        tokens.pop();
    }
    Ok(tokens)
}

fn quoted_token(
    chars: &[char],
    start: usize,
    opening: char,
    closing: char,
) -> Result<(String, usize), &'static str> {
    debug_assert_eq!(chars[start], opening);
    let mut value = String::new();
    let mut index = start + 1;
    while index < chars.len() {
        if chars[index] == closing {
            if opening != '[' && chars.get(index + 1) == Some(&closing) {
                value.push(closing);
                index += 2;
                continue;
            }
            return Ok((value, index + 1));
        }
        value.push(chars[index]);
        index += 1;
    }
    Err("unterminated SQL quote")
}

fn is_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
}

fn is_sql_keyword(word: &str) -> bool {
    matches!(
        word,
        "as" | "asc"
            | "cascade"
            | "collate"
            | "constraint"
            | "create"
            | "default"
            | "deferrable"
            | "delete"
            | "desc"
            | "foreign"
            | "index"
            | "initially"
            | "integer"
            | "key"
            | "match"
            | "no"
            | "not"
            | "null"
            | "on"
            | "primary"
            | "references"
            | "table"
            | "text"
            | "unique"
            | "unindexed"
            | "update"
            | "using"
            | "virtual"
    )
}

/// Whether `table`'s `PRAGMA table_xinfo` rows equal `expected` exactly:
/// the same number of columns, in the same declaration order, with the
/// same case-sensitive name, exact catalog-normalized declared type,
/// `NOT NULL` flag, default value, and hidden/generated flag for each.
/// Unlike `table_info`, `table_xinfo` exposes generated/hidden columns, so
/// an extra column cannot evade the exact count. SQLite normalizes type casing
/// in this pragma, so casing is not semantic; an affinity-compatible but
/// differently declared type such as `VARCHAR` or `INT` remains incompatible.
fn columns_exact(
    connection: &Connection,
    table: &str,
    expected: &[ColumnSpec],
) -> Result<bool, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT name, \"type\", \"notnull\", dflt_value, hidden FROM pragma_table_xinfo(?1) \
             ORDER BY cid",
        )
        .map_err(|_| probe_error())?;
    let rows = statement
        .query_map(rusqlite::params![table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|_| probe_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| probe_error())?;
    if rows.len() != expected.len() {
        return Ok(false);
    }
    for ((name, declared, notnull, default, hidden), column) in rows.iter().zip(expected.iter()) {
        if name != column.name
            || declared.to_ascii_uppercase() != column.declared_type
            || (*notnull != 0) != column.not_null
            || default.as_deref() != column.default
            || *hidden != column.hidden
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether `table`'s `PRAGMA table_xinfo` `pk` sequence numbers mark
/// exactly `expected` as its ordered primary-key columns (and nothing
/// else).
fn primary_key_exact(
    connection: &Connection,
    table: &str,
    expected: &[(&str, bool)],
) -> Result<bool, HistoryError> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_xinfo(?1) WHERE pk <> 0 ORDER BY pk")
        .map_err(|_| probe_error())?;
    let rows = statement
        .query_map(rusqlite::params![table], |row| row.get::<_, String>(0))
        .map_err(|_| probe_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| probe_error())?;
    if rows.len() != expected.len()
        || !rows
            .iter()
            .zip(expected.iter())
            .all(|(actual, (wanted, _))| actual == wanted)
    {
        return Ok(false);
    }
    let indexes = indexes_with_origin(connection, table, "pk")?;
    Ok(indexes.len() == 1
        && index_semantics_exact(connection, table, &indexes[0], true, "pk", expected)?)
}

/// Whether `table` owns exactly the expected set of non-primary-key
/// unique indexes: exactly `expected.len()` catalog entries with
/// constraint origin `u`, and each one covering exactly one expected
/// ordered column list. A named `CREATE UNIQUE INDEX` reports creation
/// origin `c`, so it cannot masquerade as a table uniqueness constraint;
/// the contracted named indexes are separately proven non-unique in
/// [`named_index_exact`].
fn unique_indexes_exact(
    connection: &Connection,
    table: &str,
    expected: &[&[&str]],
) -> Result<bool, HistoryError> {
    let index_names = indexes_with_origin(connection, table, "u")?;
    if index_names.len() != expected.len() {
        return Ok(false);
    }
    // Each expected column list must be covered by exactly one of the
    // table's unique indexes; since both counts are equal and no two
    // contracted rules share one column list, a full bipartite match on
    // distinct indexes is guaranteed by a per-rule search.
    let mut covered = vec![false; index_names.len()];
    for columns in expected {
        let mut matched = false;
        for (position, index_name) in index_names.iter().enumerate() {
            if covered[position] {
                continue;
            }
            let expected = columns
                .iter()
                .map(|name| (*name, false))
                .collect::<Vec<_>>();
            if index_semantics_exact(connection, table, index_name, true, "u", &expected)? {
                covered[position] = true;
                matched = true;
                break;
            }
        }
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether the named index on `table` exists exactly once, is non-unique
/// with creation origin `c` (a schema-created named index, never a
/// sneaked-in constraint), and covers exactly `expected` ordered
/// `(column, descending)` pairs.
fn named_index_exact(
    connection: &Connection,
    table: &str,
    index: &str,
    expected: &[(&str, bool)],
) -> Result<bool, HistoryError> {
    index_semantics_exact(connection, table, index, false, "c", expected)
}

/// Every index name on `table` having exactly `origin`.
fn indexes_with_origin(
    connection: &Connection,
    table: &str,
    origin: &str,
) -> Result<Vec<String>, HistoryError> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_index_list(?1) WHERE origin = ?2 ORDER BY seq")
        .map_err(|_| probe_error())?;
    statement
        .query_map(rusqlite::params![table, origin], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|_| probe_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| probe_error())
}

/// Whether one contracted index has its complete catalog semantics: one
/// owning-table `PRAGMA index_list` row with the required uniqueness and
/// origin and `partial = 0`, plus exactly the required ordered `key = 1`
/// rows from `PRAGMA index_xinfo`. Every key is a real named column (never
/// an expression), uses SQLite's contracted `BINARY` collation, and has
/// the required ascending/descending direction. Auxiliary `key = 0`
/// payload rows are not part of the key.
fn index_semantics_exact(
    connection: &Connection,
    table: &str,
    index: &str,
    unique: bool,
    origin: &str,
    expected: &[(&str, bool)],
) -> Result<bool, HistoryError> {
    let entries: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_index_list(?1)
             WHERE name = ?2 AND \"unique\" = ?3 AND origin = ?4 AND partial = 0",
            rusqlite::params![table, index, i64::from(unique), origin],
            |row| row.get(0),
        )
        .map_err(|_| probe_error())?;
    if entries != 1 {
        return Ok(false);
    }

    let mut statement = connection
        .prepare(
            "SELECT name, \"desc\", coll
             FROM pragma_index_xinfo(?1)
             WHERE key = 1
             ORDER BY seqno",
        )
        .map_err(|_| probe_error())?;
    let keys = statement
        .query_map(rusqlite::params![index], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, i64>(1)? != 0,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|_| probe_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| probe_error())?;
    Ok(keys.len() == expected.len()
        && keys.iter().zip(expected.iter()).all(
            |((name, descending, collation), (expected_name, expected_descending))| {
                name.as_deref() == Some(*expected_name)
                    && *descending == *expected_descending
                    && collation.as_deref() == Some("BINARY")
            },
        ))
}

/// Whether `table`'s foreign-key list equals `expected` exactly: the same
/// number of entries, and every expected `(from, target table, target
/// column, on_update, on_delete, match)` rule present. The expected order
/// within one table is irrelevant (SQLite assigns ids), but no expected
/// rule may be missing and no unexpected rule may exist. When `expected`
/// is empty the table must declare no foreign keys at all.
fn foreign_keys_exact(
    connection: &Connection,
    table: &str,
    expected: &[ForeignKeySpec],
) -> Result<bool, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT \"table\", \"from\", \"to\", on_update, on_delete, \"match\" \
             FROM pragma_foreign_key_list(?1) ORDER BY id, seq",
        )
        .map_err(|_| probe_error())?;
    let rows = statement
        .query_map(rusqlite::params![table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|_| probe_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| probe_error())?;
    if rows.len() != expected.len() {
        return Ok(false);
    }
    Ok(expected.iter().all(|rule| {
        rows.iter()
            .any(|(ref_table, from, to, on_update, on_delete, matching)| {
                from == rule.0
                    && ref_table == rule.1
                    && to == rule.2
                    && on_update == rule.3
                    && on_delete == rule.4
                    && matching == rule.5
            })
    }))
}

/// Whether `analysis_search` exposes the exact FTS5 column surface schema v1
/// locks. The complete canonical catalog comparison above separately proves
/// the virtual-table declaration and all options exactly, including the
/// tokenizer and `UNINDEXED` marker.
fn fts5_exact(connection: &Connection) -> Result<bool, HistoryError> {
    let mut statement = connection
        .prepare("SELECT name, hidden FROM pragma_table_xinfo('analysis_search') ORDER BY cid")
        .map_err(|_| probe_error())?;
    let columns = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|_| probe_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| probe_error())?;
    const EXPECTED: [(&str, i64); 7] = [
        ("analysis_id", 0),
        ("input_text", 0),
        ("filename", 0),
        ("headline", 0),
        ("source_urls", 0),
        ("analysis_search", 1),
        ("rank", 1),
    ];
    Ok(columns.len() == EXPECTED.len()
        && columns
            .iter()
            .zip(EXPECTED.iter())
            .all(|(actual, wanted)| actual.0 == wanted.0 && actual.1 == wanted.1))
}
