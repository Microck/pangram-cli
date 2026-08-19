use std::path::Path;
use std::str::FromStr as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::get;
use microck_pangram_cli::domain::Sha256Hash;
use microck_pangram_cli::domain::UtcTimestamp;
use microck_pangram_cli::update::{
    ArchiveFormat, DirectReplacement, DirectUpdateCandidate, InstallManager, ReleaseDecision,
    Target, TrustedManifestKey, UpdateCheckKind, UpdateChecker, UpdateErrorKind, UpdateState,
    detect_manager_install, finalize_pending_receipt, load_update_state, production_manifest_keys,
    store_update_state, validate_archive, validate_install_receipt, verify_manifest,
};
use serde_json::{Value, json};
use tar::EntryType;

#[cfg(unix)]
#[path = "update-contract/direct-installer.rs"]
mod direct_installer;
#[path = "update-contract/fixtures.rs"]
mod fixtures;

use fixtures::{KEY_ID, artifact_for, manifest, signed, tar_xz, zip};

#[test]
fn production_key_ring_binds_the_release_environment_public_key() {
    let keys = production_manifest_keys();

    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key_id(), "pangram-release-2026-01");
    assert_eq!(
        keys[0].public_key(),
        &[
            0xbb, 0x21, 0x97, 0x24, 0x90, 0xc1, 0x32, 0xe7, 0xf4, 0x9c, 0xaa, 0xd9, 0xb2, 0x5d,
            0x1d, 0x7e, 0x6a, 0xc3, 0xc2, 0x54, 0x0b, 0xc4, 0x27, 0x60, 0x9b, 0xe7, 0x92, 0x11,
            0x43, 0xa1, 0x26, 0x42,
        ]
    );
}

#[test]
fn verifies_the_exact_downloaded_bytes_before_parsing_the_manifest() {
    let manifest_bytes = serde_json::to_vec(&manifest()).unwrap();
    let (signature, key) = signed(&manifest_bytes, KEY_ID);

    let verified =
        verify_manifest(&manifest_bytes, &signature, std::slice::from_ref(&key)).unwrap();
    assert_eq!(verified.version().to_string(), "1.2.0");

    let mut reformatted = manifest_bytes.clone();
    reformatted.push(b'\n');
    let error = verify_manifest(&reformatted, &signature, &[key]).unwrap_err();
    assert_eq!(error.kind(), UpdateErrorKind::ManifestSignature);
}

#[test]
fn signature_failure_precedes_manifest_parsing() {
    let invalid_json = b"not JSON";
    let (valid_signature, key) = signed(invalid_json, KEY_ID);
    let parse_error =
        verify_manifest(invalid_json, &valid_signature, std::slice::from_ref(&key)).unwrap_err();
    assert_eq!(parse_error.kind(), UpdateErrorKind::ManifestInvalid);

    let (wrong_signature, _) = signed(b"different bytes", KEY_ID);
    let signature_error = verify_manifest(invalid_json, &wrong_signature, &[key]).unwrap_err();
    assert_eq!(signature_error.kind(), UpdateErrorKind::ManifestSignature);
}

#[test]
fn rejects_unknown_removed_keys_but_accepts_an_overlap_key_ring() {
    let manifest_bytes = serde_json::to_vec(&manifest()).unwrap();
    let (signature, current_key) = signed(&manifest_bytes, KEY_ID);
    let old_key = TrustedManifestKey::new("fixture-2025", [9; 32]);

    verify_manifest(&manifest_bytes, &signature, &[old_key.clone(), current_key]).unwrap();

    let error = verify_manifest(&manifest_bytes, &signature, &[old_key]).unwrap_err();
    assert_eq!(error.kind(), UpdateErrorKind::UnknownManifestKey);
}

#[test]
fn validates_the_signed_manifest_contract_after_signature_verification() {
    for mutate in [
        |value: &mut Value| value["schema_version"] = json!("2"),
        |value: &mut Value| value["channel"] = json!("preview"),
        |value: &mut Value| value["notes_url"] = json!("http://example.test/notes"),
        |value: &mut Value| value["artifacts"][1]["target"] = json!("x86_64-unknown-linux-gnu"),
    ] {
        let mut value = manifest();
        mutate(&mut value);
        let bytes = serde_json::to_vec(&value).unwrap();
        let (signature, key) = signed(&bytes, KEY_ID);
        let error = verify_manifest(&bytes, &signature, &[key]).unwrap_err();
        assert_eq!(error.kind(), UpdateErrorKind::ManifestInvalid, "{value}");
    }
}

#[test]
fn applies_version_and_target_policy_without_downgrade_or_guessing() {
    let bytes = serde_json::to_vec(&manifest()).unwrap();
    let (signature, key) = signed(&bytes, KEY_ID);
    let verified = verify_manifest(&bytes, &signature, &[key]).unwrap();

    let update = verified
        .release_for("1.1.0", "1.0.0", Target::X86_64UnknownLinuxGnu)
        .unwrap();
    assert!(matches!(update, ReleaseDecision::Update(_)));

    assert!(matches!(
        verified
            .release_for("1.2.0", "1.0.0", Target::X86_64UnknownLinuxGnu)
            .unwrap(),
        ReleaseDecision::NoUpdate
    ));

    let downgrade = verified
        .release_for("1.3.0", "1.0.0", Target::X86_64UnknownLinuxGnu)
        .unwrap_err();
    assert_eq!(downgrade.kind(), UpdateErrorKind::Downgrade);

    let old_updater = verified
        .release_for("1.1.0", "0.9.0", Target::X86_64UnknownLinuxGnu)
        .unwrap_err();
    assert_eq!(old_updater.kind(), UpdateErrorKind::UpdaterTooOld);

    let missing_target = verified
        .release_for("1.1.0", "1.0.0", Target::Aarch64AppleDarwin)
        .unwrap_err();
    assert_eq!(missing_target.kind(), UpdateErrorKind::TargetUnavailable);
}

#[test]
fn validates_tar_xz_and_zip_archives_and_extracts_only_the_executable() {
    let linux_executable = b"linux executable";
    let linux_archive = tar_xz(&[
        ("pangram", linux_executable, EntryType::Regular),
        ("README.md", b"readme", EntryType::Regular),
        ("LICENSE", b"license", EntryType::Regular),
        ("completions/pangram.bash", b"complete", EntryType::Regular),
        ("man/pangram.1", b"manual", EntryType::Regular),
    ]);
    let linux_artifact = artifact_for(
        &linux_archive,
        linux_executable.len(),
        Target::X86_64UnknownLinuxGnu,
        ArchiveFormat::TarXz,
    );
    assert_eq!(
        validate_archive(&linux_artifact, &linux_archive).unwrap(),
        linux_executable
    );

    let windows_executable = b"windows executable";
    let windows_archive = zip(&[
        ("pangram.exe", windows_executable),
        ("README.md", b"readme"),
        ("LICENSE", b"license"),
        ("completions/pangram.ps1", b"complete"),
        ("man/pangram.1", b"manual"),
    ]);
    let windows_artifact = artifact_for(
        &windows_archive,
        windows_executable.len(),
        Target::X86_64PcWindowsMsvc,
        ArchiveFormat::Zip,
    );
    assert_eq!(
        validate_archive(&windows_artifact, &windows_archive).unwrap(),
        windows_executable
    );
}

#[test]
fn rejects_archive_size_hash_expanded_size_and_layout_mismatches() {
    let executable = b"executable";
    let archive = tar_xz(&[("pangram", executable, EntryType::Regular)]);
    let valid = artifact_for(
        &archive,
        executable.len(),
        Target::X86_64UnknownLinuxGnu,
        ArchiveFormat::TarXz,
    );

    let mut truncated = archive.clone();
    truncated.pop();
    assert_eq!(
        validate_archive(&valid, &truncated).unwrap_err().kind(),
        UpdateErrorKind::ArchiveSize
    );

    let mut tampered = archive.clone();
    let middle = tampered.len() / 2;
    tampered[middle] ^= 1;
    assert_eq!(
        validate_archive(&valid, &tampered).unwrap_err().kind(),
        UpdateErrorKind::ArchiveHash
    );

    let wrong_expanded_size = artifact_for(
        &archive,
        executable.len() + 1,
        Target::X86_64UnknownLinuxGnu,
        ArchiveFormat::TarXz,
    );
    assert_eq!(
        validate_archive(&wrong_expanded_size, &archive)
            .unwrap_err()
            .kind(),
        UpdateErrorKind::ArchiveLayout
    );

    let unexpected = tar_xz(&[
        ("pangram", executable, EntryType::Regular),
        ("notes.txt", b"not allowed", EntryType::Regular),
    ]);
    let unexpected_artifact = artifact_for(
        &unexpected,
        executable.len(),
        Target::X86_64UnknownLinuxGnu,
        ArchiveFormat::TarXz,
    );
    assert_eq!(
        validate_archive(&unexpected_artifact, &unexpected)
            .unwrap_err()
            .kind(),
        UpdateErrorKind::ArchiveLayout
    );
}

#[test]
fn rejects_duplicate_executables_and_non_file_entries() {
    let duplicate = tar_xz(&[
        ("pangram", b"first", EntryType::Regular),
        ("pangram", b"second", EntryType::Regular),
    ]);
    let duplicate_artifact = artifact_for(
        &duplicate,
        5,
        Target::X86_64UnknownLinuxGnu,
        ArchiveFormat::TarXz,
    );
    assert_eq!(
        validate_archive(&duplicate_artifact, &duplicate)
            .unwrap_err()
            .kind(),
        UpdateErrorKind::ArchiveLayout
    );

    for entry_type in [
        EntryType::Symlink,
        EntryType::Link,
        EntryType::Char,
        EntryType::Block,
    ] {
        let archive = tar_xz(&[("pangram", b"", entry_type)]);
        let artifact = artifact_for(
            &archive,
            1,
            Target::X86_64UnknownLinuxGnu,
            ArchiveFormat::TarXz,
        );
        assert_eq!(
            validate_archive(&artifact, &archive).unwrap_err().kind(),
            UpdateErrorKind::ArchiveLayout,
            "{entry_type:?}"
        );
    }
}

#[test]
fn a_direct_receipt_must_match_the_exact_executable_version_and_target() {
    let receipt = json!({
        "schema_version": "1",
        "method": "direct",
        "executable_path": "/home/example/.local/bin/pangram",
        "installed_version": "1.2.0",
        "target": "x86_64-unknown-linux-gnu",
        "manifest_sha256": "3333333333333333333333333333333333333333333333333333333333333333",
        "installed_at": "2026-08-25T00:00:00Z"
    });
    let bytes = serde_json::to_vec(&receipt).unwrap();
    validate_install_receipt(
        &bytes,
        Path::new("/home/example/.local/bin/pangram"),
        "1.2.0",
        Target::X86_64UnknownLinuxGnu,
    )
    .unwrap();

    for (path, version, target) in [
        (
            "/home/example/bin/pangram",
            "1.2.0",
            Target::X86_64UnknownLinuxGnu,
        ),
        (
            "/home/example/.local/bin/pangram",
            "1.1.0",
            Target::X86_64UnknownLinuxGnu,
        ),
        (
            "/home/example/.local/bin/pangram",
            "1.2.0",
            Target::Aarch64UnknownLinuxGnu,
        ),
    ] {
        let error = validate_install_receipt(&bytes, Path::new(path), version, target).unwrap_err();
        assert_eq!(error.kind(), UpdateErrorKind::InstallNotOwned);
    }

    let mut malformed = receipt;
    malformed["method"] = json!("homebrew");
    let error = validate_install_receipt(
        &serde_json::to_vec(&malformed).unwrap(),
        Path::new("/home/example/.local/bin/pangram"),
        "1.2.0",
        Target::X86_64UnknownLinuxGnu,
    )
    .unwrap_err();
    assert_eq!(error.kind(), UpdateErrorKind::InstallReceiptInvalid);
}

#[test]
fn known_package_manager_paths_return_advice_without_claiming_update_ownership() {
    for (path, manager, command) in [
        (
            "/opt/homebrew/Cellar/pangram/1.2.0/bin/pangram",
            InstallManager::Homebrew,
            "brew upgrade pangram",
        ),
        (
            r"C:\Users\example\scoop\apps\pangram\current\pangram.exe",
            InstallManager::Scoop,
            "scoop update pangram",
        ),
        (
            "/usr/local/lib/node_modules/@microck/pangram-cli-linux-x64/bin/pangram",
            InstallManager::Npm,
            "npm update --global @microck/pangram-cli",
        ),
        (
            "/usr/local/lib/node_modules/@microck/pangram-cli-linux-arm64/bin/pangram",
            InstallManager::Npm,
            "npm update --global @microck/pangram-cli",
        ),
        (
            "/usr/local/lib/node_modules/@microck/pangram-cli-darwin-x64/bin/pangram",
            InstallManager::Npm,
            "npm update --global @microck/pangram-cli",
        ),
        (
            "/usr/local/lib/node_modules/@microck/pangram-cli-darwin-arm64/bin/pangram",
            InstallManager::Npm,
            "npm update --global @microck/pangram-cli",
        ),
        (
            r"C:\Users\example\AppData\Roaming\npm\node_modules\@microck\pangram-cli-win32-x64\bin\pangram.exe",
            InstallManager::Npm,
            "npm update --global @microck/pangram-cli",
        ),
    ] {
        let advisory = detect_manager_install(Path::new(path)).unwrap();
        assert_eq!(advisory.manager(), manager);
        assert_eq!(advisory.command(), command);
    }
    for path in [
        "/home/example/.local/bin/pangram",
        "/usr/local/lib/node_modules/@microck/pangram-cli/bin/pangram",
        "/usr/local/lib/node_modules/@microck/pangram-cli-linux-x64-extra/bin/pangram",
    ] {
        assert!(detect_manager_install(Path::new(path)).is_none(), "{path}");
    }
}

#[test]
fn update_state_obeys_the_24_hour_interval_and_clock_rollback_rules() {
    let checked_at = UtcTimestamp::from_str("2026-08-25T00:00:00Z").unwrap();
    let before = UtcTimestamp::from_str("2026-08-24T23:59:59Z").unwrap();
    let too_soon = UtcTimestamp::from_str("2026-08-25T23:59:59Z").unwrap();
    let due = UtcTimestamp::from_str("2026-08-26T00:00:00Z").unwrap();
    let state =
        UpdateState::checked(checked_at, Some("\"etag-1\"".into()), Some("1.2.0".into())).unwrap();

    assert!(state.should_check(before));
    assert!(!state.should_check(too_soon));
    assert!(state.should_check(due));

    let not_modified = state.not_modified(due);
    assert_eq!(not_modified.etag(), Some("\"etag-1\""));
    assert_eq!(not_modified.available_version(), Some("1.2.0"));
    assert_eq!(not_modified.last_checked_at(), due);

    let no_update = UpdateState::checked(due, Some("\"etag-2\"".into()), None).unwrap();
    assert_eq!(no_update.available_version(), None);
    let value = serde_json::to_value(no_update).unwrap();
    assert!(value.get("available_version").is_none());
}

#[cfg(unix)]
#[test]
fn update_state_roundtrips_atomically_with_owner_only_permissions() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().unwrap();
    let checked_at = UtcTimestamp::from_str("2026-08-25T00:00:00Z").unwrap();
    let state = UpdateState::checked(
        checked_at,
        Some("\"fixture-etag\"".into()),
        Some("1.2.0".into()),
    )
    .unwrap();

    assert!(load_update_state(root.path()).unwrap().is_none());
    store_update_state(root.path(), &state).unwrap();
    let path = root.path().join("update-state.json");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(load_update_state(root.path()).unwrap(), Some(state));

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        load_update_state(root.path()).unwrap_err().kind(),
        UpdateErrorKind::UpdateStateInvalid
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_check_uses_etag_and_preserves_state_on_not_modified() {
    let manifest_bytes = serde_json::to_vec(&manifest()).unwrap();
    let (signature_bytes, key) = signed(&manifest_bytes, KEY_ID);
    let manifest_requests = Arc::new(AtomicUsize::new(0));
    let signature_requests = Arc::new(AtomicUsize::new(0));
    let manifest_counter = Arc::clone(&manifest_requests);
    let signature_counter = Arc::clone(&signature_requests);
    let served_manifest = manifest_bytes.clone();
    let served_signature = signature_bytes.clone();

    let app = Router::new()
        .route(
            "/manifest.json",
            get(move |headers: HeaderMap| {
                let manifest = served_manifest.clone();
                let counter = Arc::clone(&manifest_counter);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    if headers
                        .get(header::IF_NONE_MATCH)
                        .is_some_and(|value| value == "\"fixture-etag\"")
                    {
                        return (StatusCode::NOT_MODIFIED, HeaderMap::new(), Bytes::new());
                    }
                    let mut response_headers = HeaderMap::new();
                    response_headers.insert(header::ETAG, "\"fixture-etag\"".parse().unwrap());
                    (StatusCode::OK, response_headers, Bytes::from(manifest))
                }
            }),
        )
        .route(
            "/manifest.json.sig",
            get(move || {
                let signature = served_signature.clone();
                let counter = Arc::clone(&signature_counter);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::OK, Bytes::from(signature))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let checker = UpdateChecker::for_test(
        format!("http://{address}/manifest.json"),
        format!("http://{address}/manifest.json.sig"),
    )
    .unwrap();

    let first_time = UtcTimestamp::from_str("2026-08-25T00:00:00Z").unwrap();
    let first = checker
        .check(
            None,
            first_time,
            "1.1.0",
            "1.0.0",
            Target::X86_64UnknownLinuxGnu,
            std::slice::from_ref(&key),
        )
        .await
        .unwrap();
    assert_eq!(first.kind(), UpdateCheckKind::UpdateAvailable);
    assert_eq!(first.state().etag(), Some("\"fixture-etag\""));
    assert_eq!(first.state().available_version(), Some("1.2.0"));

    let second_time = UtcTimestamp::from_str("2026-08-26T00:00:00Z").unwrap();
    let second = checker
        .check(
            Some(first.state()),
            second_time,
            "1.1.0",
            "1.0.0",
            Target::X86_64UnknownLinuxGnu,
            &[key],
        )
        .await
        .unwrap();
    assert_eq!(second.kind(), UpdateCheckKind::NotModified);
    assert_eq!(second.state().etag(), Some("\"fixture-etag\""));
    assert_eq!(second.state().available_version(), Some("1.2.0"));
    assert_eq!(second.state().last_checked_at(), second_time);
    assert_eq!(manifest_requests.load(Ordering::SeqCst), 2);
    assert_eq!(signature_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn verification_failure_returns_without_mutating_prior_state() {
    let manifest_bytes = serde_json::to_vec(&manifest()).unwrap();
    let (wrong_signature, key) = signed(b"different manifest", KEY_ID);
    let app = Router::new()
        .route(
            "/manifest.json",
            get(move || {
                let manifest = manifest_bytes.clone();
                async move { (StatusCode::OK, Bytes::from(manifest)) }
            }),
        )
        .route(
            "/manifest.json.sig",
            get(move || {
                let signature = wrong_signature.clone();
                async move { (StatusCode::OK, Bytes::from(signature)) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let checker = UpdateChecker::for_test(
        format!("http://{address}/manifest.json"),
        format!("http://{address}/manifest.json.sig"),
    )
    .unwrap();
    let original = UpdateState::checked(
        UtcTimestamp::from_str("2026-08-24T00:00:00Z").unwrap(),
        Some("\"old\"".into()),
        Some("1.1.0".into()),
    )
    .unwrap();
    let preserved = original.clone();

    let error = checker
        .check(
            Some(&original),
            UtcTimestamp::from_str("2026-08-25T00:00:00Z").unwrap(),
            "1.0.0",
            "1.0.0",
            Target::X86_64UnknownLinuxGnu,
            &[key],
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), UpdateErrorKind::ManifestSignature);
    assert_eq!(original, preserved);
}

#[cfg(unix)]
#[test]
fn direct_update_replaces_atomically_only_after_candidate_smoke_test() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("pangram");
    let receipt_path = root.path().join("install-receipt.json");
    let old_version = "0.0.0";
    let old_program = b"#!/bin/sh\nprintf 'pangram 0.0.0\\n'\n";
    fs::write(&executable, old_program).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let old_receipt = json!({
        "schema_version": "1",
        "method": "direct",
        "executable_path": executable.to_str().unwrap(),
        "installed_version": old_version,
        "target": "x86_64-unknown-linux-gnu",
        "manifest_sha256": "4444444444444444444444444444444444444444444444444444444444444444",
        "installed_at": "2026-08-24T00:00:00Z"
    });
    fs::write(&receipt_path, serde_json::to_vec(&old_receipt).unwrap()).unwrap();
    fs::set_permissions(&receipt_path, fs::Permissions::from_mode(0o600)).unwrap();
    // Success evidence uses the same native executable shape as a release.
    // Shell fixtures remain appropriate in the negative tests that need to
    // force an exact smoke mismatch.
    let new_version = env!("CARGO_PKG_VERSION");
    let new_program = fs::read(env!("CARGO_BIN_EXE_pangram")).unwrap();
    let manifest_hash =
        Sha256Hash::from_str("5555555555555555555555555555555555555555555555555555555555555555")
            .unwrap();

    let replacement = microck_pangram_cli::update::replace_direct_install(
        &executable,
        &receipt_path,
        old_version,
        Target::X86_64UnknownLinuxGnu,
        DirectUpdateCandidate::new(
            &new_program,
            new_version,
            manifest_hash,
            UtcTimestamp::from_str("2026-08-25T00:00:00Z").unwrap(),
        ),
    )
    .unwrap();
    let DirectReplacement::Completed(receipt) = replacement else {
        panic!("Unix replacement completes before returning");
    };
    assert_eq!(fs::read(&executable).unwrap(), new_program);
    assert_eq!(receipt.installed_version(), new_version);
    assert_eq!(
        fs::metadata(&executable).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(&receipt_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    validate_install_receipt(
        &fs::read(&receipt_path).unwrap(),
        &executable,
        new_version,
        Target::X86_64UnknownLinuxGnu,
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn successful_replacement_can_finalize_a_pending_receipt_without_replacing_again() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("pangram");
    let receipt_path = root.path().join("install-receipt.json");
    let old_program = b"#!/bin/sh\nprintf 'pangram 1.0.0\\n'\n";
    fs::write(&executable, old_program).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let old_receipt = serde_json::to_vec(&json!({
        "schema_version": "1",
        "method": "direct",
        "executable_path": executable.to_str().unwrap(),
        "installed_version": "1.0.0",
        "target": "x86_64-unknown-linux-gnu",
        "manifest_sha256": "8888888888888888888888888888888888888888888888888888888888888888",
        "installed_at": "2026-08-24T00:00:00Z"
    }))
    .unwrap();
    fs::write(&receipt_path, &old_receipt).unwrap();
    fs::set_permissions(&receipt_path, fs::Permissions::from_mode(0o600)).unwrap();

    // The installed-path smoke test removes directory write access after the
    // executable has been replaced, forcing only receipt publication to fail.
    let new_program = format!(
        "#!/bin/sh\ncase \"$0\" in *.update-*) ;; *) if [ ! -e '{}'/smoked ]; then touch '{}'/smoked; chmod 500 '{}'; fi;; esac\nprintf 'pangram 1.1.0\\n'\n",
        root.path().display(),
        root.path().display(),
        root.path().display(),
    );
    let manifest_hash =
        Sha256Hash::from_str("9999999999999999999999999999999999999999999999999999999999999999")
            .unwrap();
    let error = microck_pangram_cli::update::replace_direct_install(
        &executable,
        &receipt_path,
        "1.0.0",
        Target::X86_64UnknownLinuxGnu,
        DirectUpdateCandidate::new(
            new_program.as_bytes(),
            "1.1.0",
            manifest_hash,
            UtcTimestamp::from_str("2026-08-25T00:00:00Z").unwrap(),
        ),
    )
    .unwrap_err();
    assert_eq!(error.kind(), UpdateErrorKind::ReplaceFailed);
    assert_eq!(fs::read(&executable).unwrap(), new_program.as_bytes());
    assert_eq!(fs::read(&receipt_path).unwrap(), old_receipt);

    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_file(root.path().join("smoked")).unwrap();
    fs::write(root.path().join("smoked"), b"already failed once").unwrap();
    let finalized = finalize_pending_receipt(
        &executable,
        &receipt_path,
        "1.1.0",
        Target::X86_64UnknownLinuxGnu,
    )
    .unwrap();
    assert_eq!(finalized.installed_version(), "1.1.0");
    validate_install_receipt(
        &fs::read(&receipt_path).unwrap(),
        &executable,
        "1.1.0",
        Target::X86_64UnknownLinuxGnu,
    )
    .unwrap();
}
