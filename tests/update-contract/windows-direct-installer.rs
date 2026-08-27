//! Windows direct-installer replacement behavior.

use std::fs;
use std::str::FromStr as _;

use microck_pangram_cli::domain::{Sha256Hash, UtcTimestamp};
use microck_pangram_cli::update::{
    DirectReplacement, DirectUpdateCandidate, Target, install_direct_candidate,
    validate_install_receipt,
};

#[test]
fn archive_candidate_replacement_finishes_before_returning() {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("bin/pangram.exe");
    let receipt_path = root.path().join("data/install-receipt.json");
    let version = env!("CARGO_PKG_VERSION");
    let program = fs::read(env!("CARGO_BIN_EXE_pangram")).unwrap();

    let initial = install_direct_candidate(
        &executable,
        &receipt_path,
        Target::X86_64PcWindowsMsvc,
        DirectUpdateCandidate::new(
            &program,
            version,
            Sha256Hash::from_str(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
            UtcTimestamp::from_str("2026-08-26T00:00:00Z").unwrap(),
        ),
    )
    .unwrap();
    assert!(matches!(initial, DirectReplacement::Completed(_)));
    let initial_receipt = fs::read(&receipt_path).unwrap();

    let replacement = install_direct_candidate(
        &executable,
        &receipt_path,
        Target::X86_64PcWindowsMsvc,
        DirectUpdateCandidate::new(
            &program,
            version,
            Sha256Hash::from_str(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .unwrap(),
            UtcTimestamp::from_str("2026-08-26T00:00:01Z").unwrap(),
        ),
    )
    .unwrap();
    let DirectReplacement::Completed(receipt) = replacement else {
        panic!("an archive candidate must finish replacement before returning");
    };

    let replacement_receipt = fs::read(&receipt_path).unwrap();
    assert_ne!(replacement_receipt, initial_receipt);
    assert_eq!(receipt.installed_at().to_string(), "2026-08-26T00:00:01Z");
    validate_install_receipt(
        &replacement_receipt,
        &executable,
        version,
        Target::X86_64PcWindowsMsvc,
    )
    .unwrap();
    assert_eq!(fs::read(&executable).unwrap(), program);
    assert_eq!(
        fs::read_dir(receipt_path.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}
