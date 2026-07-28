use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use microck_pangram_cli::contracts::{
    GeneratedArtifact, generated_artifacts, write_generated_artifacts,
};

const EXPECTED_ARTIFACTS: &[&str] = &[
    "contracts/config.schema.json",
    "contracts/install-receipt.schema.json",
    "contracts/manifest-signature.schema.json",
    "contracts/output.schema.json",
    "contracts/tui-state.schema.json",
    "contracts/update-manifest.schema.json",
    "contracts/update-state.schema.json",
    "generated/cli-help.txt",
    "generated/cli-reference.json",
    "generated/error-reference.json",
];

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect_files(root: &Path, directory: &Path, files: &mut BTreeSet<String>) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            collect_files(root, &entry.path(), files);
        } else {
            assert!(
                file_type.is_file(),
                "{} is not a regular file",
                entry.path().display()
            );
            let relative = entry.path().strip_prefix(root).unwrap().to_path_buf();
            files.insert(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn generated_artifact_inventory_is_complete_and_unique() {
    let artifacts = generated_artifacts().unwrap();
    let actual: BTreeSet<_> = artifacts
        .iter()
        .map(|artifact| artifact.path.as_str())
        .collect();
    let expected: BTreeSet<_> = EXPECTED_ARTIFACTS.iter().copied().collect();

    assert_eq!(actual, expected);
}

#[test]
fn owned_directories_contain_exactly_the_generated_inventory() {
    let root = repository_root();
    let mut actual = BTreeSet::new();
    collect_files(&root, &root.join("contracts"), &mut actual);
    collect_files(&root, &root.join("generated"), &mut actual);
    let expected = generated_artifacts()
        .unwrap()
        .into_iter()
        .map(|artifact| artifact.path)
        .collect();

    assert_eq!(actual, expected);
}

#[test]
fn staging_failure_does_not_replace_earlier_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let artifacts = generated_artifacts().unwrap();
    for artifact in artifacts
        .iter()
        .filter(|artifact| artifact.path.starts_with("contracts/"))
    {
        let path = root.path().join(&artifact.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"previous contract").unwrap();
    }
    fs::write(root.path().join("generated"), b"blocks directory creation").unwrap();

    assert!(write_generated_artifacts(root.path()).is_err());
    for artifact in artifacts
        .iter()
        .filter(|artifact| artifact.path.starts_with("contracts/"))
    {
        assert_eq!(
            fs::read(root.path().join(&artifact.path)).unwrap(),
            b"previous contract"
        );
    }
}

#[test]
fn committed_contracts_match_rust_owned_generation() {
    let root = repository_root();

    for GeneratedArtifact { path, bytes } in generated_artifacts().unwrap() {
        let committed = fs::read(root.join(&path))
            .unwrap_or_else(|error| panic!("failed to read {path}: {error}"));
        assert_eq!(
            committed, bytes,
            "{path} differs from its Rust-owned generator"
        );
    }
}

#[test]
fn every_json_artifact_declares_generated_rust_ownership() {
    for GeneratedArtifact { path, bytes } in generated_artifacts().unwrap() {
        if Path::new(&path)
            .extension()
            .and_then(|value| value.to_str())
            != Some("json")
        {
            continue;
        }

        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value
                .get("x-contract-owner")
                .and_then(|owner| owner.as_str()),
            Some("rust:microck_pangram_cli::contracts")
        );
    }
}
