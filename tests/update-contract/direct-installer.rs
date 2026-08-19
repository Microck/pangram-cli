//! Initial direct-install behavior and failure preservation.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::str::FromStr as _;

use microck_pangram_cli::domain::{Sha256Hash, UtcTimestamp};
use microck_pangram_cli::update::{
    DirectReplacement, DirectUpdateCandidate, Target, UpdateErrorKind, install_direct_candidate,
    replace_direct_install, validate_install_receipt,
};
use serde_json::json;

#[test]
fn creates_the_executable_and_receipt_only_after_smoke() {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("bin/pangram");
    let receipt_path = root.path().join("data/install-receipt.json");
    let version = env!("CARGO_PKG_VERSION");
    let program = fs::read(env!("CARGO_BIN_EXE_pangram")).unwrap();
    let manifest_hash =
        Sha256Hash::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap();

    let replacement = install_direct_candidate(
        &executable,
        &receipt_path,
        Target::X86_64UnknownLinuxGnu,
        DirectUpdateCandidate::new(
            &program,
            version,
            manifest_hash,
            UtcTimestamp::from_str("2026-08-26T00:00:00Z").unwrap(),
        ),
    )
    .unwrap();
    let DirectReplacement::Completed(receipt) = replacement else {
        panic!("a new direct install completes in the installer process");
    };

    assert_eq!(fs::read(&executable).unwrap(), program);
    assert_eq!(receipt.installed_version(), version);
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
        version,
        Target::X86_64UnknownLinuxGnu,
    )
    .unwrap();
}

#[test]
fn refuses_an_unowned_existing_executable() {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("pangram");
    let receipt_path = root.path().join("install-receipt.json");
    let original = b"#!/bin/sh\nprintf 'pangram 9.9.9\\n'\n";
    fs::write(&executable, original).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

    let error = install_direct_candidate(
        &executable,
        &receipt_path,
        Target::X86_64UnknownLinuxGnu,
        DirectUpdateCandidate::new(
            b"#!/bin/sh\nprintf 'pangram 1.1.0\\n'\n",
            "1.1.0",
            Sha256Hash::from_str(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .unwrap(),
            UtcTimestamp::from_str("2026-08-26T00:00:00Z").unwrap(),
        ),
    )
    .unwrap_err();

    assert_eq!(error.kind(), UpdateErrorKind::InstallNotOwned);
    assert_eq!(fs::read(&executable).unwrap(), original);
    assert!(!receipt_path.exists());
}

#[test]
fn failed_candidate_smoke_preserves_executable_and_receipt_bytes() {
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
        "manifest_sha256": "6666666666666666666666666666666666666666666666666666666666666666",
        "installed_at": "2026-08-24T00:00:00Z"
    }))
    .unwrap();
    fs::write(&receipt_path, &old_receipt).unwrap();
    fs::set_permissions(&receipt_path, fs::Permissions::from_mode(0o600)).unwrap();

    let error = replace_direct_install(
        &executable,
        &receipt_path,
        "1.0.0",
        Target::X86_64UnknownLinuxGnu,
        DirectUpdateCandidate::new(
            b"#!/bin/sh\nprintf 'pangram 9.9.9\\n'\n",
            "1.1.0",
            Sha256Hash::from_str(
                "7777777777777777777777777777777777777777777777777777777777777777",
            )
            .unwrap(),
            UtcTimestamp::from_str("2026-08-25T00:00:00Z").unwrap(),
        ),
    )
    .unwrap_err();

    assert_eq!(error.kind(), UpdateErrorKind::ReplaceFailed);
    assert_eq!(fs::read(&executable).unwrap(), old_program);
    assert_eq!(fs::read(&receipt_path).unwrap(), old_receipt);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
