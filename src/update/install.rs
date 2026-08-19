//! Hidden direct-installer candidate mode.
//!
//! Generated shell installers fetch bytes and verify the archive identity
//! embedded at release time. The archived candidate enters this mode to apply
//! the stronger Rust-owned signature, archive, self-identity, and receipt
//! checks before any destination mutation.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use super::{
    DirectUpdateCandidate, Target, UpdateError, UpdateErrorKind, install_direct_candidate,
    production_manifest_keys, validate_archive, verify_manifest,
};
use crate::config::{ConfigOverrides, Paths};
use crate::domain::Sha256Hash;

const INSTALL_RECEIPT_FILE_NAME: &str = "install-receipt.json";
const INSTALL_MODE: &str = "__pangram-direct-install";

/// Handles the candidate-only installer mode before public Clap parsing.
/// Every normal invocation returns `None`.
pub(crate) fn run_direct_install_helper<I, T>(arguments: I) -> Option<u8>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut arguments = arguments.into_iter().map(Into::into);
    let _program = arguments.next();
    if arguments.next().as_deref().and_then(OsStr::to_str) != Some(INSTALL_MODE) {
        return None;
    }
    let arguments = arguments.collect::<Vec<_>>();
    Some(match install(&arguments) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("pangram installer: {error}");
            1
        }
    })
}

fn install(arguments: &[OsString]) -> Result<(), UpdateError> {
    if arguments.len() != 8 {
        return Err(installer_error());
    }
    let manifest_path = argument_value(arguments, "--manifest", 0)?;
    let signature_path = argument_value(arguments, "--signature", 2)?;
    let archive_path = argument_value(arguments, "--archive", 4)?;
    let destination = PathBuf::from(argument_value(arguments, "--destination", 6)?);
    if !destination.is_absolute() {
        return Err(installer_error());
    }

    let manifest_bytes = read_regular_file(Path::new(manifest_path))?;
    let signature_bytes = read_regular_file(Path::new(signature_path))?;
    let archive_bytes = read_regular_file(Path::new(archive_path))?;
    let manifest = verify_manifest(
        &manifest_bytes,
        &signature_bytes,
        &production_manifest_keys(),
    )?;
    if manifest.version() != env!("CARGO_PKG_VERSION") {
        return Err(installer_error());
    }
    let target = Target::current().ok_or_else(installer_error)?;
    let artifact = manifest
        .artifacts()
        .iter()
        .find(|artifact| artifact.target() == target)
        .ok_or_else(installer_error)?;
    let executable_bytes = validate_archive(artifact, &archive_bytes)?;

    // The program that holds installation authority must be the exact root
    // executable extracted from the verified archive, not another local copy.
    let current_executable = std::env::current_exe().map_err(|_| installer_error())?;
    if !file_matches(&current_executable, &executable_bytes)? {
        return Err(installer_error());
    }

    let paths = Paths::resolve(&ConfigOverrides::default()).map_err(|_| installer_error())?;
    let receipt_path = paths.platform_data_dir().join(INSTALL_RECEIPT_FILE_NAME);
    install_direct_candidate(
        &destination,
        &receipt_path,
        target,
        DirectUpdateCandidate::new(
            &executable_bytes,
            manifest.version(),
            Sha256Hash::digest(&manifest_bytes),
            crate::domain::UtcTimestamp::now(),
        ),
    )?;
    Ok(())
}

fn argument_value<'a>(
    arguments: &'a [OsString],
    flag: &str,
    index: usize,
) -> Result<&'a OsStr, UpdateError> {
    if arguments.get(index).and_then(|value| value.to_str()) != Some(flag) {
        return Err(installer_error());
    }
    arguments
        .get(index + 1)
        .map(OsString::as_os_str)
        .ok_or_else(installer_error)
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>, UpdateError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| installer_error())?;
    if !metadata.file_type().is_file() {
        return Err(installer_error());
    }
    fs::read(path).map_err(|_| installer_error())
}

fn file_matches(path: &Path, expected: &[u8]) -> Result<bool, UpdateError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| installer_error())?;
    if !metadata.file_type().is_file()
        || metadata.len() != u64::try_from(expected.len()).map_err(|_| installer_error())?
    {
        return Ok(false);
    }
    let mut file = fs::File::open(path).map_err(|_| installer_error())?;
    let mut buffer = [0_u8; 64 * 1024];
    for expected_chunk in expected.chunks(buffer.len()) {
        let buffer_chunk = &mut buffer[..expected_chunk.len()];
        file.read_exact(buffer_chunk)
            .map_err(|_| installer_error())?;
        if buffer_chunk != expected_chunk {
            return Ok(false);
        }
    }
    let mut trailing = [0_u8; 1];
    file.read(&mut trailing)
        .map(|read| read == 0)
        .map_err(|_| installer_error())
}

const fn installer_error() -> UpdateError {
    UpdateError::new(
        UpdateErrorKind::ReplaceFailed,
        "The direct installer could not validate or install this release.",
    )
}
