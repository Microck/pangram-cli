#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::process::Command;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signer as _, SigningKey};
use microck_pangram_cli::update::{
    ReleaseDecision, Target, TrustedManifestKey, validate_archive, verify_manifest,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

#[test]
fn release_packager_emits_an_archive_accepted_by_the_shipping_updater() {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("pangram");
    let executable_bytes = b"#!/bin/sh\ncase \"$1\" in completions) printf 'completion for %s\\n' \"$2\";; --help) printf 'Pangram fixture help\\n';; esac\n";
    fs::write(&executable, executable_bytes).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_package-release"))
        .args([
            "--target",
            "x86_64-unknown-linux-gnu",
            "--executable",
            executable.to_str().unwrap(),
            "--out-dir",
            root.path().to_str().unwrap(),
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .unwrap();
    assert!(status.success());

    let metadata: Value = serde_json::from_slice(
        &fs::read(root.path().join("x86_64-unknown-linux-gnu.artifact.json")).unwrap(),
    )
    .unwrap();
    let archive = fs::read(root.path().join(metadata["file_name"].as_str().unwrap())).unwrap();
    let manifest = json!({
        "schema_version": "1",
        "channel": "stable",
        "version": "1.0.0",
        "published_at": "2026-08-25T00:00:00Z",
        "notes_url": "https://github.com/Microck/pangram-cli/releases/tag/v1.0.0",
        "minimum_updater_version": "0.1.0",
        "artifacts": [{
            "target": metadata["target"],
            "archive_format": metadata["archive_format"],
            "url": "https://github.com/Microck/pangram-cli/releases/download/v1.0.0/pangram-v1.0.0-x86_64-unknown-linux-gnu.tar.xz",
            "size_bytes": metadata["size_bytes"],
            "executable_size_bytes": metadata["executable_size_bytes"],
            "sha256": metadata["sha256"]
        }]
    });
    let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    let signing_key = SigningKey::from_bytes(&[13; 32]);
    let signature = signing_key.sign(&bytes);
    let signature_document = serde_json::to_vec(&json!({
        "schema_version": "1",
        "algorithm": "ed25519",
        "key_id": "release-packaging-fixture",
        "signature": STANDARD.encode(signature.to_bytes())
    }))
    .unwrap();
    let verified = verify_manifest(
        &bytes,
        &signature_document,
        &[TrustedManifestKey::new(
            "release-packaging-fixture",
            signing_key.verifying_key().to_bytes(),
        )],
    )
    .unwrap();
    let artifact = match verified
        .release_for("0.1.0", "0.1.0", Target::X86_64UnknownLinuxGnu)
        .unwrap()
    {
        ReleaseDecision::Update(artifact) => artifact,
        ReleaseDecision::NoUpdate => panic!("fixture release must be newer"),
    };
    let extracted = validate_archive(artifact, &archive).unwrap();
    assert_eq!(extracted, executable_bytes);
}

#[test]
fn manifest_builder_signs_a_stable_zero_major_five_target_set() {
    let root = tempfile::tempdir().unwrap();
    let artifacts = root.path().join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    for (target, format) in [
        ("aarch64-apple-darwin", "tar.xz"),
        ("aarch64-unknown-linux-gnu", "tar.xz"),
        ("x86_64-apple-darwin", "tar.xz"),
        ("x86_64-unknown-linux-gnu", "tar.xz"),
        ("x86_64-pc-windows-msvc", "zip"),
    ] {
        let file_name = format!("pangram-v0.1.0-{target}.{format}");
        let bytes = format!("fixture archive for {target}").into_bytes();
        fs::write(artifacts.join(&file_name), &bytes).unwrap();
        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        fs::write(
            artifacts.join(format!("{target}.artifact.json")),
            serde_json::to_vec_pretty(&json!({
                "target": target,
                "archive_format": format,
                "file_name": file_name,
                "size_bytes": bytes.len(),
                "executable_size_bytes": 1,
                "sha256": sha256
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let key_file = root.path().join("fixture-key");
    fs::write(&key_file, [17; 32]).unwrap();
    fs::set_permissions(&key_file, fs::Permissions::from_mode(0o600)).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_sign-update-manifest"))
        .args([
            "--artifacts-dir",
            artifacts.to_str().unwrap(),
            "--out-dir",
            artifacts.to_str().unwrap(),
            "--key-file",
            key_file.to_str().unwrap(),
            "--key-id",
            "fixture-release-key",
            "--version",
            "0.1.0",
            "--published-at",
            "2026-08-25T00:00:00Z",
            "--minimum-updater-version",
            "1.0.0",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let manifest = fs::read(artifacts.join("pangram-update-manifest.json")).unwrap();
    let signature = fs::read(artifacts.join("pangram-update-manifest.json.sig")).unwrap();
    let signing_key = SigningKey::from_bytes(&[17; 32]);
    let verified = verify_manifest(
        &manifest,
        &signature,
        &[TrustedManifestKey::new(
            "fixture-release-key",
            signing_key.verifying_key().to_bytes(),
        )],
    )
    .unwrap();
    assert_eq!(verified.version(), "0.1.0");
    assert_eq!(verified.artifacts().len(), 5);

    let status = Command::new("node")
        .args([
            "scripts/render-package-manifests.mjs",
            artifacts.to_str().unwrap(),
            "0.1.0",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .unwrap();
    assert!(status.success());
    let posix_installer = fs::read_to_string(artifacts.join("pangram-installer.sh")).unwrap();
    let powershell_installer = fs::read_to_string(artifacts.join("pangram-installer.ps1")).unwrap();
    for installer in [&posix_installer, &powershell_installer] {
        assert!(installer.contains("__pangram-direct-install"));
        assert!(installer.contains("pangram-update-manifest.json.sig"));
        assert!(installer.contains("0.1.0"));
        assert!(!installer.contains("{{"));
    }
    assert!(posix_installer.contains("releases/download/v0.1.0"));
    assert!(powershell_installer.contains("releases/download/v$version"));
    assert!(!posix_installer.contains("releases/latest/download"));
    assert!(!powershell_installer.contains("releases/latest/download"));
    for asset in [
        "pangram-update-manifest.json",
        "pangram-update-manifest.json.sig",
    ] {
        assert!(posix_installer.contains(&format!("$release_url/{asset}")));
        assert!(powershell_installer.contains(&format!("$releaseUrl/{asset}")));
    }
    assert!(posix_installer.contains("x86_64-unknown-linux-gnu"));
    assert!(powershell_installer.contains("x86_64-pc-windows-msvc"));
    assert_eq!(
        fs::metadata(artifacts.join("pangram-installer.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}
