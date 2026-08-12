//! Compiled-binary bulk-submit integration against the real loopback Pangram
//! 4 fixture. No mocks, no live Pangram, no real credentials.
//!
//! Each test boots the Axum fixture on an ephemeral loopback port, points the
//! development-only compiled test driver at it (one loopback
//! set derives both the task and bulk routes), runs the compiled `pangram`
//! binary in an isolated config/data environment, and asserts the exact
//! stdout envelope, stderr separation, exit code, and the recorded upstream
//! request grammar. The synthetic key and content are fixture constants;
//! assertion helpers never echo header or key values.
//!
//! Contract coverage (contracts.md 3.1, 9.2, 14.3): the dry-run
//! reconciliation envelope matches the generated closed `bulk_submit` union;
//! ceiling and whole-file input preflights run before credentials or network
//! and never bill; and the recorded POST/GET routes prove exactly one
//! billable send with no replay.

#![cfg(feature = "dev-tools")]

#[path = "support/bulk_cli_env.rs"]
mod env;

use std::io::Write as _;
use std::process::Stdio;

use serde_json::Value;

use env::fixture::{BulkRequestView, ProtocolFixture, SYNTHETIC_KEY, Step};
use env::{
    BULK_ID, Isolated, accepted_202, accepted_202_split, assert_no_leak, interrupt, jsonl,
    spawn_with_stdin, stdout_envelope,
};

// A valid two-source dry run validates the plan, projects the canonical
// closed `bulk_submit` union's dry-run shape, and never touches the fixture.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_dry_run_emits_typed_json_without_key_or_network() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let input = jsonl(&[("row-001", "first synthetic words"), ("row-002", "second")]);
    let output = spawn_with_stdin(
        isolated.command_without_key(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--dry-run",
        ],
        input.as_bytes(),
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, b"", "a dry run is silent on stderr");
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["schema_version"], "1");
    assert_eq!(envelope["command"], "bulk_submit");
    assert!(envelope.get("error").is_none());
    let data = &envelope["data"];
    // The canonical dry-run marker and reconciliation tuple.
    assert_eq!(data["dry"]["noop"], true);
    assert_eq!(data["dry"]["observed"], false);
    assert!(
        data["id"].as_str().unwrap().starts_with("bulk_"),
        "a fresh local bulk ID: {data}"
    );
    assert!(
        data.get("upstream_bulk_id").is_none(),
        "a dry run has no remote identity"
    );
    assert_eq!(data["status"], "queued");
    assert_eq!(data["submission_outcome"], "not_submitted");
    assert_eq!(data["estimated_billable_units"], 2);
    assert_eq!(data["item_count"], 2);
    assert!(
        data["plan_sha256"].as_str().unwrap().len() == 64,
        "a lowercase SHA-256: {data}"
    );
    assert!(envelope["meta"]["started_at"].is_string());
    assert_eq!(fixture.post_count() + fixture.get_count(), 0, "no network");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// The dry-run shape validates against the generated closed `bulk_submit`
// union (a `BulkCollection` OR `BulkDryRun`), proving the schema owns it.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_dry_run_matches_the_generated_closed_union() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let input = jsonl(&[("row-001", "first synthetic words")]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--dry-run",
        ],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);

    let schema_bytes = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts/output.schema.json"),
    )
    .expect("read the committed output schema");
    let schema: Value = serde_json::from_slice(&schema_bytes).unwrap();
    let validator = jsonschema::validator_for(&schema).expect("the output schema compiles");
    assert!(
        validator.is_valid(&envelope),
        "the dry-run envelope validates against the closed union"
    );
    fixture.shutdown().await;
}

// A `--dry-run` with a non-JSON format is rejected before credentials or the
// network: the reconciliation shape has no TOON/pretty projection.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_dry_run_rejects_a_non_json_format_early() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let input = jsonl(&[("row-001", "first synthetic words")]);
    let output = spawn_with_stdin(
        isolated.command_without_key(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--dry-run",
            "--format",
            "pretty",
        ],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(2), "a usage error, before work");
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "unsupported_combination");
    assert_eq!(fixture.post_count() + fixture.get_count(), 0);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A submitted (non-dry-run) `bulk submit` with a non-JSON `--format` is
// rejected as `unsupported_combination` before any source read, plan
// validation, credential resolution, or the billable POST. The hoisted guard
// runs before the flow prepares anything, so even a scripted valid 202 is
// never reached: zero POST and zero GET reach the fixture.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_submitted_rejects_a_non_json_format_before_any_post() {
    let fixture = ProtocolFixture::start().await;
    // A valid acceptance is scripted; the guard must fire before it is ever
    // sent, proving the rejection precedes the billable POST.
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_202(1))));
    let isolated = Isolated::new();
    let input = jsonl(&[("row-000", "first synthetic words")]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--format",
            "pretty",
        ],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(2), "a usage error, before work");
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "unsupported_combination");
    assert_eq!(
        fixture.post_count() + fixture.get_count(),
        0,
        "no billable send and no read reach the network"
    );
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// `--dry-run --wait` is an unsupported combination, before any work.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_dry_run_rejects_wait() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let input = jsonl(&[("row-001", "first synthetic words")]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--dry-run",
            "--wait",
        ],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "unsupported_combination");
    assert_eq!(fixture.post_count() + fixture.get_count(), 0);
    fixture.shutdown().await;
}

// `--dry-run --wait` is an unsupported combination, before any work.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_requires_max_billable_units() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let input = jsonl(&[("row-001", "words")]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &["bulk", "submit", "-", "--dry-run"],
        input.as_bytes(),
    );
    // Clap owns the required-flag usage surface.
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fixture.post_count() + fixture.get_count(), 0);
    fixture.shutdown().await;
}

// A zero ceiling is a usage error before credentials or network.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_rejects_a_zero_ceiling_before_work() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let input = jsonl(&[("row-001", "words")]);
    let output = spawn_with_stdin(
        isolated.command_without_key(fixture.base_url()),
        &["bulk", "submit", "-", "--max-billable-units", "0"],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "unsupported_input");
    assert_eq!(fixture.post_count() + fixture.get_count(), 0);
    fixture.shutdown().await;
}

// An estimate over the 1000-unit request ceiling fails fast, before work.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_rejects_an_estimate_over_the_request_cap() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    // 101 words over 501 items -> 2 units per item = 1002 > 1000-unit cap,
    // so rejection comes from the request cap, not the caller ceiling (set
    // above the cap). The started-100-word rule bills 101 words as 2 units.
    let text = "word ".repeat(101);
    let owned: Vec<(String, String)> = (0..501)
        .map(|index| (format!("row-{index:03}"), text.clone()))
        .collect();
    let refs: Vec<(&str, &str)> = owned
        .iter()
        .map(|(id, words)| (id.as_str(), words.as_str()))
        .collect();
    let input = jsonl(&refs);
    let output = spawn_with_stdin(
        isolated.command_without_key(fixture.base_url()),
        &["bulk", "submit", "-", "--max-billable-units", "2000"],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "bulk_limit_exceeded");
    assert_eq!(fixture.post_count() + fixture.get_count(), 0, "no billing");
    fixture.shutdown().await;
}

// An estimate above the caller ceiling but below the cap fails before work.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_rejects_an_estimate_over_the_caller_ceiling() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let words = "word ".repeat(101); // 101 words -> 2 units
    let input = jsonl(&[("row-001", words.trim_end())]);
    let output = spawn_with_stdin(
        isolated.command_without_key(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "1",
            "--dry-run",
        ],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "bulk_limit_exceeded");
    assert_eq!(fixture.post_count() + fixture.get_count(), 0);
    fixture.shutdown().await;
}

// UTF-8 non-ASCII text is accepted and priced by the whitespace word count.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_accepts_utf8_and_counts_words() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    // Accented Latin input (cafe/, resume/, nai\"ve) exercises real
    // non-ASCII UTF-8 at runtime while the source file stays ASCII.
    let text = "caf\u{e9} r\u{e9}sum\u{e9} na\u{ef}ve";
    let input = jsonl(&[("row-001", text)]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--dry-run",
        ],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["item_count"], 1);
    assert_eq!(envelope["data"]["estimated_billable_units"], 1);
    fixture.shutdown().await;
}

// An empty JSONL source is the canonical input_required usage error.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_empty_stdin_is_input_required() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &["bulk", "submit", "-", "--max-billable-units", "10"],
        b"",
    );
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "input_required");
    assert_eq!(fixture.post_count() + fixture.get_count(), 0);
    fixture.shutdown().await;
}

// Invalid JSONL is a usage error naming the line, before any work.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_invalid_jsonl_is_a_usage_error() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--dry-run",
        ],
        b"{not json}\n",
    );
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "unsupported_input");
    assert_eq!(fixture.post_count() + fixture.get_count(), 0);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// Unknown item fields fail whole-file validation before work.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_rejects_unknown_item_fields() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--dry-run",
        ],
        b"{\"id\":\"row-001\",\"text\":\"words\",\"extra\":true}\n",
    );
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "unsupported_input");
    assert_eq!(fixture.post_count() + fixture.get_count(), 0);
    fixture.shutdown().await;
}

// Duplicate caller IDs fail whole-file validation before work.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_rejects_duplicate_caller_ids() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--dry-run",
        ],
        b"{\"id\":\"dup\",\"text\":\"one\"}\n{\"id\":\"dup\",\"text\":\"two\"}\n",
    );
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "unsupported_input");
    assert_eq!(fixture.post_count() + fixture.get_count(), 0);
    fixture.shutdown().await;
}

// A submitted job accepted with 202 reports the queued collection, exits 0,
// and sends exactly one POST with the documented job-wide model grammar.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_accepted_reports_queued_collection_and_one_post() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_202(2))));
    let isolated = Isolated::new();
    let input = jsonl(&[
        ("row-000", "first synthetic words"),
        ("row-001", "second words"),
    ]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &["bulk", "submit", "-", "--max-billable-units", "10"],
        input.as_bytes(),
    );

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_submit");
    let data = &envelope["data"];
    assert_eq!(data["status"], "queued");
    assert_eq!(data["submission_outcome"], "accepted");
    assert_eq!(data["upstream_bulk_id"], BULK_ID);
    assert_eq!(data["total_items"], 2);
    assert_eq!(data["estimated_billable_units"], 2);

    let recorded = fixture.requests();
    let submits = BulkRequestView::submits(&recorded);
    assert_eq!(submits.len(), 1, "exactly one billable POST");
    assert!(submits[0].header_equals("x-api-key", SYNTHETIC_KEY));
    let sent = submits[0].body_json();
    assert_eq!(sent["model"], "pangram-4");
    assert_eq!(sent["items"].as_array().unwrap().len(), 2);
    assert_eq!(sent["items"][0]["id"], "row-000");
    assert!(sent.get("public_dashboard_link").is_none());
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.get_count(), 0, "no polling without --wait");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A 202 that accepts every item projects the truthful accepted snapshot:
// queued with all items accepted, none yet finished, exit 0, one POST.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_without_wait_projects_all_accepted_snapshot() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_202(3))));
    let isolated = Isolated::new();
    let input = jsonl(&[
        ("row-000", "first synthetic words"),
        ("row-001", "second synthetic words"),
        ("row-002", "third synthetic words"),
    ]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &["bulk", "submit", "-", "--max-billable-units", "10"],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0), "a parsed 202 exits 0");
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_submit");
    let data = &envelope["data"];
    assert_eq!(data["status"], "queued");
    assert_eq!(data["submission_outcome"], "accepted");
    assert_eq!(data["upstream_bulk_id"], BULK_ID);
    assert_eq!(data["total_items"], 3);
    assert_eq!(data["accepted"], 3);
    assert_eq!(data["succeeded"], 0);
    assert_eq!(data["failed"], 0);
    assert!(data.get("completed_at").is_none(), "non-terminal");
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.get_count(), 0, "no polling without --wait");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A 202 that accepts some items and immediately fails others through
// immediate upstream validation still exits 0 (the acceptance itself is the
// authority), while reporting the truthful counters: accepted and failed
// counts from the 202, not fabricated all-queued-zero values.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_without_wait_projects_mixed_acceptance_truthfully() {
    let fixture = ProtocolFixture::start().await;
    // 3 items, 2 accepted + 1 immediately failed.
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_202_split(3, 2))));
    let isolated = Isolated::new();
    let input = jsonl(&[
        ("row-000", "first synthetic words"),
        ("row-001", "second synthetic words"),
        ("row-002", "third synthetic words"),
    ]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &["bulk", "submit", "-", "--max-billable-units", "10"],
        input.as_bytes(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a parsed 202 with immediate failures still exits 0"
    );
    let envelope = stdout_envelope(&output);
    let data = &envelope["data"];
    // Some accepted work remains, so the collection is queued (non-terminal).
    assert_eq!(data["status"], "queued");
    assert_eq!(data["total_items"], 3);
    assert_eq!(data["accepted"], 2);
    assert_eq!(data["succeeded"], 0);
    assert_eq!(data["failed"], 1);
    assert_eq!(data["submission_outcome"], "accepted");
    assert_eq!(fixture.post_count(), 1, "one billable send, no replay");
    assert_eq!(fixture.get_count(), 0, "no polling without --wait");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A 202 that immediately rejects every submitted item is still a parsed
// acceptance (exit 0), but the truthful snapshot is the terminal `failed`
// collection (all items failed), never a fabricated all-queued-zero state.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_without_wait_projects_an_all_failed_acceptance() {
    let fixture = ProtocolFixture::start().await;
    // 2 items, both immediately failed by upstream validation.
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_202_split(2, 0))));
    let isolated = Isolated::new();
    let input = jsonl(&[
        ("row-000", "first synthetic words"),
        ("row-001", "second synthetic words"),
    ]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &["bulk", "submit", "-", "--max-billable-units", "10"],
        input.as_bytes(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a parsed 202 that rejected every item still exits 0"
    );
    let envelope = stdout_envelope(&output);
    let data = &envelope["data"];
    assert_eq!(data["status"], "failed");
    assert_eq!(data["total_items"], 2);
    assert_eq!(data["accepted"], 0);
    assert_eq!(data["succeeded"], 0);
    assert_eq!(data["failed"], 2);
    assert_eq!(data["submission_outcome"], "accepted");
    assert_eq!(fixture.post_count(), 1, "one billable send, no replay");
    assert_eq!(fixture.get_count(), 0, "no polling without --wait");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A JSONL file path (not stdin) reads and submits the same way.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_reads_a_jsonl_file_path() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_202(1))));
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("items.jsonl");
    std::fs::write(&path, jsonl(&[("row-000", "file words here")])).unwrap();
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args([
            "bulk",
            "submit",
            path.to_str().unwrap(),
            "--max-billable-units",
            "10",
        ])
        .output()
        .expect("run bulk submit with a file path");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["upstream_bulk_id"], BULK_ID);
    assert_eq!(fixture.post_count(), 1);
    fixture.shutdown().await;
}

// A 413 on submit maps to bulk_limit_exceeded with sanitized 413 detail.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_413_maps_to_bulk_limit_exceeded() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(413, None, None));
    let isolated = Isolated::new();
    let input = jsonl(&[("row-000", "words")]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &["bulk", "submit", "-", "--max-billable-units", "10"],
        input.as_bytes(),
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "bulk_limit_exceeded is usage"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "bulk_limit_exceeded");
    assert_eq!(fixture.post_count(), 1);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A 401 on submit maps to the authentication exit with an invalid_api_key
// envelope and never echoes the key.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_401_maps_to_invalid_api_key() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(401, None, None));
    let isolated = Isolated::new();
    let input = jsonl(&[("row-000", "words")]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &["bulk", "submit", "-", "--max-billable-units", "10"],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(4));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "invalid_api_key");
    assert_eq!(fixture.post_count(), 1);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// An ambiguous billable POST (issued then interrupted) is reported as
// submission_outcome_unknown, never replayed, exiting 130 with the
// reconciliation identity on stderr.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_ambiguous_post_reports_outcome_unknown_without_replay() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Hang);
    let isolated = Isolated::new();
    let input = jsonl(&[("row-000", "words whose send is lost")]);
    let mut child = isolated
        .command(fixture.base_url())
        .args(["bulk", "submit", "-", "--max-billable-units", "10"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn bulk submit");
    // Feed the source and close stdin so the CLI reads EOF, plans, and
    // submits; wait until the billable POST reaches the fixture so the send
    // is unambiguously issued, then interrupt.
    let mut stdin = child.stdin.take().expect("piped stdin");
    stdin.write_all(input.as_bytes()).unwrap();
    drop(stdin);
    fixture.wait_for_posts(1).await;
    interrupt(&mut child);
    let output = child.wait_with_output().expect("await bulk submit");

    assert_eq!(output.status.code(), Some(130), "interrupted by the user");
    assert_eq!(
        fixture.post_count(),
        1,
        "the ambiguous send is never replayed"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "submission_outcome_unknown");
    assert_eq!(envelope["error"]["retryable"], false);
    assert_eq!(
        envelope["error"]["recovery"]["message"],
        "A manual retry may create a second billable operation."
    );
    assert!(envelope["error"]["details"]["bulk_id"].is_string());
    assert!(envelope["error"]["details"]["request_sha256"].is_string());
    assert_no_leak(&output);
    fixture.shutdown().await;
}
