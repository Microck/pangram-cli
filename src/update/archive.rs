//! Signed archive integrity and safe executable extraction.

use std::io::{Cursor, Read as _};
use std::path::{Component, Path};

use xz2::read::XzDecoder;

use super::{ArchiveFormat, UpdateArtifact, UpdateError, UpdateErrorKind};

/// Validates the signed archive size, digest, entry layout, and expanded
/// executable size, returning only the executable bytes.
pub fn validate_archive(
    artifact: &UpdateArtifact,
    archive_bytes: &[u8],
) -> Result<Vec<u8>, UpdateError> {
    if u64::try_from(archive_bytes.len()).ok() != Some(artifact.size_bytes) {
        return Err(UpdateError::new(
            UpdateErrorKind::ArchiveSize,
            "The update archive size does not match the signed manifest.",
        ));
    }
    if crate::domain::Sha256Hash::digest(archive_bytes) != artifact.sha256 {
        return Err(UpdateError::new(
            UpdateErrorKind::ArchiveHash,
            "The update archive hash does not match the signed manifest.",
        ));
    }

    match artifact.archive_format {
        ArchiveFormat::TarXz => validate_tar_xz(artifact, archive_bytes),
        ArchiveFormat::Zip => validate_zip(artifact, archive_bytes),
    }
}

fn validate_tar_xz(
    artifact: &UpdateArtifact,
    archive_bytes: &[u8],
) -> Result<Vec<u8>, UpdateError> {
    let decoder = XzDecoder::new(Cursor::new(archive_bytes));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(|_| invalid_archive())?;
    let mut executable = None;

    for entry in entries {
        let mut entry = entry.map_err(|_| invalid_archive())?;
        let path = entry.path().map_err(|_| invalid_archive())?;
        let kind = classify_archive_path(&path, entry.header().entry_type().is_dir())?;
        let entry_type = entry.header().entry_type();
        match kind {
            ArchiveEntry::Executable => {
                if !entry_type.is_file() || executable.is_some() {
                    return Err(invalid_archive());
                }
                executable = Some(read_executable(&mut entry, artifact.executable_size_bytes)?);
            }
            ArchiveEntry::File => {
                if !entry_type.is_file() {
                    return Err(invalid_archive());
                }
            }
            ArchiveEntry::Directory => {
                if !entry_type.is_dir() {
                    return Err(invalid_archive());
                }
            }
        }
    }

    executable.ok_or_else(invalid_archive)
}

fn validate_zip(artifact: &UpdateArtifact, archive_bytes: &[u8]) -> Result<Vec<u8>, UpdateError> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(archive_bytes)).map_err(|_| invalid_archive())?;
    let mut executable = None;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| invalid_archive())?;
        let path = entry.enclosed_name().ok_or_else(invalid_archive)?;
        let kind = classify_archive_path(&path, entry.is_dir())?;
        validate_zip_file_type(&entry, kind)?;
        if kind == ArchiveEntry::Executable {
            if executable.is_some() {
                return Err(invalid_archive());
            }
            executable = Some(read_executable(&mut entry, artifact.executable_size_bytes)?);
        }
    }

    executable.ok_or_else(invalid_archive)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchiveEntry {
    Executable,
    File,
    Directory,
}

fn classify_archive_path(path: &Path, is_directory: bool) -> Result<ArchiveEntry, UpdateError> {
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str().ok_or_else(invalid_archive),
            _ => Err(invalid_archive()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = components.first().copied() else {
        return Err(invalid_archive());
    };

    if components.len() == 1 && matches!(first, "pangram" | "pangram.exe") {
        return if is_directory {
            Err(invalid_archive())
        } else {
            Ok(ArchiveEntry::Executable)
        };
    }
    if components.len() == 1 && matches!(first, "README.md" | "LICENSE") {
        return if is_directory {
            Err(invalid_archive())
        } else {
            Ok(ArchiveEntry::File)
        };
    }
    if matches!(first, "completions" | "man") {
        if is_directory {
            return Ok(ArchiveEntry::Directory);
        }
        if components.len() >= 2 {
            return Ok(ArchiveEntry::File);
        }
    }
    Err(invalid_archive())
}

fn validate_zip_file_type<R: std::io::Read + ?Sized>(
    entry: &zip::read::ZipFile<'_, R>,
    expected: ArchiveEntry,
) -> Result<(), UpdateError> {
    if entry.is_dir() != (expected == ArchiveEntry::Directory)
        || entry.is_file() == (expected == ArchiveEntry::Directory)
    {
        return Err(invalid_archive());
    }
    if let Some(mode) = entry.unix_mode() {
        let file_type = mode & 0o170000;
        let expected_type = if expected == ArchiveEntry::Directory {
            0o040000
        } else {
            0o100000
        };
        if file_type != 0 && file_type != expected_type {
            return Err(invalid_archive());
        }
    }
    Ok(())
}

fn read_executable(
    reader: &mut impl std::io::Read,
    expected_size: u64,
) -> Result<Vec<u8>, UpdateError> {
    let capacity = usize::try_from(expected_size).map_err(|_| invalid_archive())?;
    let mut executable = Vec::with_capacity(capacity);
    reader
        .take(expected_size.saturating_add(1))
        .read_to_end(&mut executable)
        .map_err(|_| invalid_archive())?;
    if u64::try_from(executable.len()).ok() != Some(expected_size) {
        return Err(invalid_archive());
    }
    Ok(executable)
}

const fn invalid_archive() -> UpdateError {
    UpdateError::new(
        UpdateErrorKind::ArchiveLayout,
        "The update archive layout is invalid.",
    )
}
