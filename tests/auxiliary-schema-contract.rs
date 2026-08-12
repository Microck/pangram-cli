#[path = "support/schema-contract.rs"]
mod support;

use serde_json::json;
use support::{Case, assert_cases};

#[test]
fn configuration_schema_is_strict_and_bounded() {
    assert_cases(
        "config.schema.json",
        vec![
            Case {
                name: "minimal config",
                instance: json!({"config_version": 1}),
                valid: true,
            },
            Case {
                name: "complete config",
                instance: json!({
                    "config_version": 1,
                    "history": {"enabled": false},
                    "tui": {"intro": "once", "keymap": "regular", "motion": "full"},
                    "updates": {"check_on_tui_start": true},
                    "network": {"max_requests_per_second": 5}
                }),
                valid: true,
            },
            Case {
                name: "unknown key",
                instance: json!({"config_version": 1, "api_key": "secret"}),
                valid: false,
            },
            Case {
                name: "rate above Pangram ceiling",
                instance: json!({
                    "config_version": 1,
                    "network": {"max_requests_per_second": 6}
                }),
                valid: false,
            },
        ],
    );
}

#[test]
fn local_state_and_update_schemas_preserve_closed_security_contracts() {
    assert_cases(
        "tui-state.schema.json",
        vec![
            Case {
                name: "seen marker",
                instance: json!({"schema_version": "1", "intro_seen": true}),
                valid: true,
            },
            Case {
                name: "unknown marker field",
                instance: json!({
                    "schema_version": "1",
                    "intro_seen": true,
                    "credential": "not-allowed"
                }),
                valid: false,
            },
        ],
    );

    let artifact = json!({
        "target": "aarch64-unknown-linux-gnu",
        "archive_format": "tar.xz",
        "url": "https://github.com/Microck/pangram-cli/releases/download/v0.1.0/pangram.tar.xz",
        "size_bytes": 100,
        "executable_size_bytes": 200,
        "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
    });
    assert_cases(
        "update-manifest.schema.json",
        vec![
            Case {
                name: "signed stable manifest value",
                instance: json!({
                    "schema_version": "1",
                    "channel": "stable",
                    "version": "0.1.0",
                    "published_at": "2026-07-23T12:00:00Z",
                    "notes_url": "https://github.com/Microck/pangram-cli/releases/tag/v0.1.0",
                    "minimum_updater_version": "0.1.0",
                    "artifacts": [artifact]
                }),
                valid: true,
            },
            Case {
                name: "production artifact requires HTTPS",
                instance: json!({
                    "schema_version": "1",
                    "channel": "stable",
                    "version": "0.1.0",
                    "published_at": "2026-07-23T12:00:00Z",
                    "notes_url": "https://github.com/Microck/pangram-cli/releases/tag/v0.1.0",
                    "minimum_updater_version": "0.1.0",
                    "artifacts": [{
                        "target": "aarch64-unknown-linux-gnu",
                        "archive_format": "tar.xz",
                        "url": "http://example.com/pangram.tar.xz",
                        "size_bytes": 100,
                        "executable_size_bytes": 200,
                        "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
                    }]
                }),
                valid: false,
            },
            Case {
                name: "manifest versions accept canonical SemVer metadata",
                instance: json!({
                    "schema_version": "1",
                    "channel": "stable",
                    "version": "0.2.0-rc.1+release.5",
                    "published_at": "2026-07-23T12:00:00Z",
                    "notes_url": "https://github.com/Microck/pangram-cli/releases/tag/v0.2.0-rc.1",
                    "minimum_updater_version": "0.1.0-alpha.1",
                    "artifacts": [artifact]
                }),
                valid: true,
            },
            Case {
                name: "manifest rejects noncanonical SemVer leading zeroes",
                instance: json!({
                    "schema_version": "1",
                    "channel": "stable",
                    "version": "0.02.0",
                    "published_at": "2026-07-23T12:00:00Z",
                    "notes_url": "https://github.com/Microck/pangram-cli/releases/tag/v0.2.0",
                    "minimum_updater_version": "0.1.0",
                    "artifacts": [artifact]
                }),
                valid: false,
            },
        ],
    );

    assert_cases(
        "manifest-signature.schema.json",
        vec![
            Case {
                name: "Ed25519 signature",
                instance: json!({
                    "schema_version": "1",
                    "algorithm": "ed25519",
                    "key_id": "release-2026",
                    "signature": "YWJj"
                }),
                valid: true,
            },
            Case {
                name: "unknown algorithm",
                instance: json!({
                    "schema_version": "1",
                    "algorithm": "rsa",
                    "key_id": "release-2026",
                    "signature": "YWJj"
                }),
                valid: false,
            },
        ],
    );

    assert_cases(
        "update-state.schema.json",
        vec![
            Case {
                name: "cached update state",
                instance: json!({
                    "schema_version": "1",
                    "last_checked_at": "2026-07-23T12:00:00Z",
                    "etag": "\"example\"",
                    "available_version": "0.2.0"
                }),
                valid: true,
            },
            Case {
                name: "offset timestamp",
                instance: json!({
                    "schema_version": "1",
                    "last_checked_at": "2026-07-23T13:00:00+01:00"
                }),
                valid: false,
            },
            Case {
                name: "cached version accepts canonical SemVer prerelease",
                instance: json!({
                    "schema_version": "1",
                    "last_checked_at": "2026-07-23T12:00:00Z",
                    "available_version": "0.2.0-rc.1"
                }),
                valid: true,
            },
            Case {
                name: "cached version rejects a leading zero",
                instance: json!({
                    "schema_version": "1",
                    "last_checked_at": "2026-07-23T12:00:00Z",
                    "available_version": "0.02.0"
                }),
                valid: false,
            },
        ],
    );

    assert_cases(
        "install-receipt.schema.json",
        vec![
            Case {
                name: "direct receipt",
                instance: json!({
                    "schema_version": "1",
                    "method": "direct",
                    "executable_path": "/home/user/.local/bin/pangram",
                    "installed_version": "0.1.0",
                    "target": "aarch64-unknown-linux-gnu",
                    "manifest_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                    "executable_sha256": "1111111111111111111111111111111111111111111111111111111111111111",
                    "installed_at": "2026-07-23T12:00:00Z"
                }),
                valid: true,
            },
            Case {
                name: "receipt requires the installed executable digest",
                instance: json!({
                    "schema_version": "1",
                    "method": "direct",
                    "executable_path": "/home/user/.local/bin/pangram",
                    "installed_version": "0.1.0",
                    "target": "aarch64-unknown-linux-gnu",
                    "manifest_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                    "installed_at": "2026-07-23T12:00:00Z"
                }),
                valid: false,
            },
            Case {
                name: "executable digest is exact lowercase SHA-256",
                instance: json!({
                    "schema_version": "1",
                    "method": "direct",
                    "executable_path": "/home/user/.local/bin/pangram",
                    "installed_version": "0.1.0",
                    "target": "aarch64-unknown-linux-gnu",
                    "manifest_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                    "executable_sha256": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                    "installed_at": "2026-07-23T12:00:00Z"
                }),
                valid: false,
            },
            Case {
                name: "receipt rejects a noncanonical installed version",
                instance: json!({
                    "schema_version": "1",
                    "method": "direct",
                    "executable_path": "/home/user/.local/bin/pangram",
                    "installed_version": "0.01.0",
                    "target": "aarch64-unknown-linux-gnu",
                    "manifest_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                    "executable_sha256": "1111111111111111111111111111111111111111111111111111111111111111",
                    "installed_at": "2026-07-23T12:00:00Z"
                }),
                valid: false,
            },
            Case {
                name: "manager receipt cannot claim direct ownership",
                instance: json!({
                    "schema_version": "1",
                    "method": "homebrew",
                    "executable_path": "/opt/homebrew/bin/pangram",
                    "installed_version": "0.1.0",
                    "target": "aarch64-apple-darwin",
                    "manifest_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                    "executable_sha256": "1111111111111111111111111111111111111111111111111111111111111111",
                    "installed_at": "2026-07-23T12:00:00Z"
                }),
                valid: false,
            },
        ],
    );
}
