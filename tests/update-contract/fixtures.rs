//! Signed manifest and archive fixtures shared by the updater contract cases.

use std::io::{Cursor, Write as _};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signer as _, SigningKey};
use microck_pangram_cli::domain::Sha256Hash;
use microck_pangram_cli::update::{
    ArchiveFormat, Target, TrustedManifestKey, UpdateArtifact, verify_manifest,
};
use serde_json::{Value, json};
use tar::{Builder as TarBuilder, EntryType, Header};
use xz2::write::XzEncoder;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

pub const KEY_ID: &str = "fixture-2026";
const SIGNING_SEED: [u8; 32] = [7; 32];

pub fn manifest() -> Value {
    json!({
        "schema_version": "1",
        "channel": "stable",
        "version": "1.2.0",
        "published_at": "2026-08-25T00:00:00Z",
        "notes_url": "https://github.com/Microck/pangram-cli/releases/tag/v1.2.0",
        "minimum_updater_version": "1.0.0",
        "artifacts": [
            {
                "target": "x86_64-unknown-linux-gnu",
                "archive_format": "tar.xz",
                "url": "https://github.com/Microck/pangram-cli/releases/download/v1.2.0/pangram-x86_64-unknown-linux-gnu.tar.xz",
                "size_bytes": 128,
                "executable_size_bytes": 64,
                "sha256": "1111111111111111111111111111111111111111111111111111111111111111"
            },
            {
                "target": "x86_64-pc-windows-msvc",
                "archive_format": "zip",
                "url": "https://github.com/Microck/pangram-cli/releases/download/v1.2.0/pangram-x86_64-pc-windows-msvc.zip",
                "size_bytes": 256,
                "executable_size_bytes": 96,
                "sha256": "2222222222222222222222222222222222222222222222222222222222222222"
            }
        ]
    })
}

pub fn signed(manifest_bytes: &[u8], key_id: &str) -> (Vec<u8>, TrustedManifestKey) {
    let signing_key = SigningKey::from_bytes(&SIGNING_SEED);
    let signature = signing_key.sign(manifest_bytes);
    let signature_document = serde_json::to_vec(&json!({
        "schema_version": "1",
        "algorithm": "ed25519",
        "key_id": key_id,
        "signature": STANDARD.encode(signature.to_bytes())
    }))
    .unwrap();
    (
        signature_document,
        TrustedManifestKey::new(KEY_ID, signing_key.verifying_key().to_bytes()),
    )
}

pub fn artifact_for(
    archive: &[u8],
    executable_size: usize,
    target: Target,
    format: ArchiveFormat,
) -> UpdateArtifact {
    let mut value = manifest();
    value["version"] = json!("1.2.1");
    value["artifacts"] = json!([{
        "target": target.as_str(),
        "archive_format": match format {
            ArchiveFormat::TarXz => "tar.xz",
            ArchiveFormat::Zip => "zip",
        },
        "url": "https://github.com/Microck/pangram-cli/releases/download/v1.2.1/archive",
        "size_bytes": archive.len(),
        "executable_size_bytes": executable_size,
        "sha256": Sha256Hash::digest(archive)
    }]);
    let bytes = serde_json::to_vec(&value).unwrap();
    let (signature, key) = signed(&bytes, KEY_ID);
    let verified = verify_manifest(&bytes, &signature, &[key]).unwrap();
    verified.artifacts().first().unwrap().clone()
}

pub fn tar_xz(entries: &[(&str, &[u8], EntryType)]) -> Vec<u8> {
    let encoder = XzEncoder::new(Vec::new(), 6);
    let mut archive = TarBuilder::new(encoder);
    for (path, body, entry_type) in entries {
        let mut header = Header::new_gnu();
        header.set_entry_type(*entry_type);
        header.set_mode(if entry_type.is_file() { 0o755 } else { 0o777 });
        header.set_size(body.len() as u64);
        header.set_cksum();
        archive.append_data(&mut header, path, *body).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap()
}

pub fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().unix_permissions(0o755);
    for (path, body) in entries {
        archive.start_file(*path, options).unwrap();
        archive.write_all(body).unwrap();
    }
    archive.finish().unwrap().into_inner()
}
