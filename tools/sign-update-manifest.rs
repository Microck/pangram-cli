use std::collections::HashSet;
use std::fs;
use std::io::{BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signer as _, SigningKey};
use microck_pangram_cli::config::read_protected_file;
use microck_pangram_cli::domain::{Sha256Hash, UtcTimestamp};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactMetadata {
    target: String,
    archive_format: String,
    file_name: String,
    size_bytes: u64,
    executable_size_bytes: u64,
    sha256: Sha256Hash,
}

#[derive(Serialize)]
struct ManifestArtifact {
    target: String,
    archive_format: String,
    url: String,
    size_bytes: u64,
    executable_size_bytes: u64,
    sha256: Sha256Hash,
}

#[derive(Serialize)]
struct Manifest {
    schema_version: &'static str,
    channel: &'static str,
    version: String,
    published_at: UtcTimestamp,
    notes_url: String,
    minimum_updater_version: String,
    artifacts: Vec<ManifestArtifact>,
}

#[derive(Serialize)]
struct SignatureDocument {
    schema_version: &'static str,
    algorithm: &'static str,
    key_id: String,
    signature: String,
}

fn main() {
    if let Err(message) = run() {
        eprintln!("sign-update-manifest: {message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let mut artifacts_dir = None;
    let mut output_dir = None;
    let mut key_file = None;
    let mut key_id = None;
    let mut version = None;
    let mut published_at = None;
    let mut minimum_updater_version = None;
    while let Some(argument) = arguments.next() {
        let value = arguments
            .next()
            .ok_or_else(|| format!("{argument} requires a value"))?;
        match argument.as_str() {
            "--artifacts-dir" => artifacts_dir = Some(PathBuf::from(value)),
            "--out-dir" => output_dir = Some(PathBuf::from(value)),
            "--key-file" => key_file = Some(PathBuf::from(value)),
            "--key-id" => key_id = Some(value),
            "--version" => version = Some(value),
            "--published-at" => published_at = Some(value),
            "--minimum-updater-version" => minimum_updater_version = Some(value),
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    let artifacts_dir = artifacts_dir.ok_or("--artifacts-dir is required")?;
    let output_dir = output_dir.ok_or("--out-dir is required")?;
    let key_file = key_file.ok_or("--key-file is required")?;
    let key_id = key_id.ok_or("--key-id is required")?;
    let version = validate_release_version(version.ok_or("--version is required")?)?;
    let minimum =
        validate_version(minimum_updater_version.ok_or("--minimum-updater-version is required")?)?;
    let published_at = UtcTimestamp::from_str(&published_at.ok_or("--published-at is required")?)
        .map_err(|_| "invalid publication timestamp")?;
    if key_id.is_empty() || !key_id.is_ascii() {
        return Err("invalid publication timestamp or key ID".into());
    }

    let entries = fs::read_dir(&artifacts_dir)
        .map_err(|_| "cannot read artifacts directory")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "cannot read artifact directory entry")?;
    let mut metadata = entries
        .into_iter()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".artifact.json")
        })
        .map(|entry| {
            let bytes = fs::read(entry.path()).map_err(|_| "cannot read artifact metadata")?;
            serde_json::from_slice::<ArtifactMetadata>(&bytes)
                .map_err(|_| "invalid artifact metadata")
        })
        .collect::<Result<Vec<_>, _>>()?;
    metadata.sort_by(|left, right| left.target.cmp(&right.target));
    let expected_targets = HashSet::from([
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ]);
    let actual_targets = metadata
        .iter()
        .map(|artifact| artifact.target.as_str())
        .collect::<HashSet<_>>();
    if metadata.len() != 5 || actual_targets != expected_targets {
        return Err("exactly five unique target metadata files are required".into());
    }
    for artifact in &metadata {
        let expected_format = if artifact.target == "x86_64-pc-windows-msvc" {
            "zip"
        } else {
            "tar.xz"
        };
        let expected_name = format!("pangram-v{version}-{}.{expected_format}", artifact.target);
        let (archive_size, digest) = hash_file(&artifacts_dir.join(&artifact.file_name))?;
        if artifact.archive_format != expected_format
            || artifact.file_name != expected_name
            || artifact.size_bytes != archive_size
            || artifact.executable_size_bytes == 0
            || artifact.sha256 != digest
        {
            return Err("artifact metadata does not match the release archive".into());
        }
    }
    let base = format!("https://github.com/Microck/pangram-cli/releases/download/v{version}");
    let artifacts = metadata
        .into_iter()
        .map(|artifact| ManifestArtifact {
            target: artifact.target,
            archive_format: artifact.archive_format,
            url: format!("{base}/{}", artifact.file_name),
            size_bytes: artifact.size_bytes,
            executable_size_bytes: artifact.executable_size_bytes,
            sha256: artifact.sha256,
        })
        .collect();
    let manifest = Manifest {
        schema_version: "1",
        channel: "stable",
        notes_url: format!("https://github.com/Microck/pangram-cli/releases/tag/v{version}"),
        version,
        published_at,
        minimum_updater_version: minimum,
        artifacts,
    };
    let mut manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|_| "cannot encode manifest")?;
    manifest_bytes.push(b'\n');

    let key_bytes = read_protected_file(&key_file).map_err(|_| "cannot read signing key")?;
    let seed = Zeroizing::new(
        key_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "signing key must contain exactly 32 raw bytes")?,
    );
    let signing_key = SigningKey::from_bytes(&seed);
    let signature = signing_key.sign(&manifest_bytes);
    let document = SignatureDocument {
        schema_version: "1",
        algorithm: "ed25519",
        key_id,
        signature: STANDARD.encode(signature.to_bytes()),
    };
    let mut signature_bytes =
        serde_json::to_vec_pretty(&document).map_err(|_| "cannot encode signature")?;
    signature_bytes.push(b'\n');

    fs::create_dir_all(&output_dir).map_err(|_| "cannot create output directory")?;
    fs::write(
        output_dir.join("pangram-update-manifest.json"),
        manifest_bytes,
    )
    .map_err(|_| "cannot write manifest")?;
    fs::write(
        output_dir.join("pangram-update-manifest.json.sig"),
        signature_bytes,
    )
    .map_err(|_| "cannot write signature")?;
    Ok(())
}

fn hash_file(path: &Path) -> Result<(u64, Sha256Hash), String> {
    let file = fs::File::open(path).map_err(|_| "cannot read release archive")?;
    let expected_size = file
        .metadata()
        .map_err(|_| "cannot inspect release archive")?
        .len();
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut actual_size = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| "cannot read release archive")?;
        if read == 0 {
            break;
        }
        actual_size = actual_size
            .checked_add(read as u64)
            .ok_or("release archive is too large")?;
        hasher.update(&buffer[..read]);
    }
    if actual_size != expected_size {
        return Err("release archive changed while it was read".into());
    }
    Ok((
        actual_size,
        Sha256Hash::from_bytes(hasher.finalize().into()),
    ))
}

fn validate_release_version(value: String) -> Result<String, String> {
    let parsed = Version::parse(&value).map_err(|_| "invalid release version")?;
    if !parsed.pre.is_empty() || !parsed.build.is_empty() {
        return Err("signed public manifests require a stable version".into());
    }
    Ok(value)
}

fn validate_version(value: String) -> Result<String, String> {
    let parsed = Version::parse(&value).map_err(|_| "invalid updater version")?;
    if !parsed.pre.is_empty() || !parsed.build.is_empty() {
        return Err("invalid updater version".into());
    }
    Ok(value)
}
