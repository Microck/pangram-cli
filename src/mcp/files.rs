//! Handle-relative access to files beneath immutable MCP-approved roots.
//!
//! Path strings select a pre-opened capability. They never authorize an
//! ambient reopen. Every directory component and the final file are opened
//! through held handles with no-follow semantics, then the opened object is
//! verified before it is returned.

use std::cmp::Reverse;
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use thiserror::Error;

/// Immutable directory capabilities approved before the MCP server reads
/// stdin.
pub(crate) struct ApprovedFileRoots {
    roots: Vec<ApprovedRoot>,
}

struct ApprovedRoot {
    path: PathBuf,
    component_count: usize,
    directory: Dir,
}

/// Failure to configure or use an approved file root.
#[derive(Debug, Error)]
pub(crate) enum ApprovedFileError {
    #[error("approved file root must be absolute: {0}")]
    RootNotAbsolute(PathBuf),
    #[error("approved file root contains an unsafe path component: {0}")]
    UnsafeRoot(PathBuf),
    #[error("failed to pre-open approved file root {path}: {source}")]
    OpenRoot {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("file path must be absolute")]
    PathNotAbsolute,
    #[error("file path contains an unsafe path component")]
    UnsafePath,
    #[error("file paths require at least one approved file root")]
    NoApprovedRoots,
    #[error("file path is outside the approved file roots")]
    OutsideApprovedRoots,
    #[error("failed to open file beneath its approved root: {0}")]
    OpenFile(#[source] io::Error),
    #[error("approved path does not identify a regular file")]
    NotRegularFile,
    #[cfg(windows)]
    #[error("approved path identifies a Windows reparse point")]
    ReparsePoint,
}

impl ApprovedFileRoots {
    /// Validates and pre-opens every root. Callers must complete this before
    /// entering the stdio protocol loop.
    pub(crate) fn preopen(paths: &[PathBuf]) -> Result<Self, ApprovedFileError> {
        let mut roots = Vec::with_capacity(paths.len());

        for path in paths {
            let normalized_path = normalize_absolute_path(path).map_err(|error| match error {
                PathShapeError::NotAbsolute => ApprovedFileError::RootNotAbsolute(path.clone()),
                PathShapeError::UnsafeComponent => ApprovedFileError::UnsafeRoot(path.clone()),
            })?;
            let components = normal_components(&normalized_path);
            let directory = preopen_root(&normalized_path, &components).map_err(|source| {
                ApprovedFileError::OpenRoot {
                    path: path.clone(),
                    source,
                }
            })?;

            roots.push(ApprovedRoot {
                path: normalized_path,
                component_count: components.len(),
                directory,
            });
        }

        // A nested approval is more specific. Stable sorting preserves the
        // caller's order for duplicate roots without changing behavior.
        roots.sort_by_key(|root| Reverse(root.component_count));

        Ok(Self { roots })
    }

    /// Opens one absolute path beneath the deepest matching approved root.
    /// The returned handle identifies the object that passed verification,
    /// even if another process later replaces its pathname.
    pub(crate) fn open(&self, path: &Path) -> Result<File, ApprovedFileError> {
        let normalized_path = normalize_absolute_path(path).map_err(|error| match error {
            PathShapeError::NotAbsolute => ApprovedFileError::PathNotAbsolute,
            PathShapeError::UnsafeComponent => ApprovedFileError::UnsafePath,
        })?;

        let root = if self.roots.is_empty() {
            return Err(ApprovedFileError::NoApprovedRoots);
        } else {
            self.roots
                .iter()
                .find(|root| normalized_path.starts_with(&root.path))
                .ok_or(ApprovedFileError::OutsideApprovedRoots)?
        };
        let relative = normalized_path
            .strip_prefix(&root.path)
            .map_err(|_| ApprovedFileError::OutsideApprovedRoots)?;

        open_relative_file(&root.directory, relative)
    }
}

#[derive(Clone, Copy)]
enum PathShapeError {
    NotAbsolute,
    UnsafeComponent,
}

fn normalize_absolute_path(path: &Path) -> Result<PathBuf, PathShapeError> {
    if !path.is_absolute() {
        return Err(PathShapeError::NotAbsolute);
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir | Component::ParentDir => {
                return Err(PathShapeError::UnsafeComponent);
            }
        }
    }
    Ok(normalized)
}

fn normal_components(path: &Path) -> Vec<OsString> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(segment) => Some(segment.to_os_string()),
            _ => None,
        })
        .collect()
}

fn preopen_root(path: &Path, components: &[OsString]) -> io::Result<Dir> {
    // Remove every normal component to obtain the platform anchor (`/`, a
    // drive root, or a UNC share), then traverse back down without following
    // links. Opening the whole ambient path would follow an intermediate link.
    let mut anchor = path;
    for _ in components {
        anchor = anchor.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "root has no filesystem anchor")
        })?;
    }

    let mut directory = Dir::open_ambient_dir(anchor, ambient_authority())?;
    verify_directory(&directory)?;
    for component in components {
        directory = directory.open_dir_nofollow(component)?;
        verify_directory(&directory)?;
    }
    Ok(directory)
}

fn open_relative_file(root: &Dir, relative: &Path) -> Result<File, ApprovedFileError> {
    let file_name = relative
        .file_name()
        .ok_or(ApprovedFileError::NotRegularFile)?;
    let mut parents = relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components();
    let Some(first_parent) = parents.next() else {
        return open_verified_file(root, file_name);
    };
    let Component::Normal(first_parent) = first_parent else {
        return Err(ApprovedFileError::UnsafePath);
    };

    let mut current = root
        .open_dir_nofollow(first_parent)
        .map_err(ApprovedFileError::OpenFile)?;
    verify_directory(&current).map_err(ApprovedFileError::OpenFile)?;
    for parent in parents {
        let Component::Normal(parent) = parent else {
            return Err(ApprovedFileError::UnsafePath);
        };
        current = current
            .open_dir_nofollow(parent)
            .map_err(ApprovedFileError::OpenFile)?;
        verify_directory(&current).map_err(ApprovedFileError::OpenFile)?;
    }

    open_verified_file(&current, file_name)
}

fn open_verified_file(
    directory: &Dir,
    file_name: &std::ffi::OsStr,
) -> Result<File, ApprovedFileError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(FollowSymlinks::No)
        // Windows otherwise rejects a directory before handle metadata can
        // classify it through the same no-follow capability path.
        .maybe_dir(true);
    let file = directory
        .open_with(file_name, &options)
        .map_err(ApprovedFileError::OpenFile)?;
    let metadata = file.metadata().map_err(ApprovedFileError::OpenFile)?;
    verify_not_reparse_point(&metadata)?;
    if !metadata.is_file() {
        return Err(ApprovedFileError::NotRegularFile);
    }

    Ok(file.into_std())
}

fn verify_directory(directory: &Dir) -> io::Result<()> {
    let metadata = directory.dir_metadata()?;
    #[cfg(windows)]
    if is_reparse_point(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory is a Windows reparse point",
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "opened object is not a directory",
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn verify_not_reparse_point(_: &cap_std::fs::Metadata) -> Result<(), ApprovedFileError> {
    Ok(())
}

#[cfg(windows)]
fn verify_not_reparse_point(metadata: &cap_std::fs::Metadata) -> Result<(), ApprovedFileError> {
    if is_reparse_point(metadata) {
        Err(ApprovedFileError::ReparsePoint)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn is_reparse_point(metadata: &cap_std::fs::Metadata) -> bool {
    use cap_std::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn read(file: File) -> String {
        std::io::read_to_string(file).unwrap()
    }

    #[test]
    fn opens_a_regular_file_through_nested_directory_handles() {
        let workspace = TempDir::new().unwrap();
        let approved = workspace.path().join("approved");
        fs::create_dir_all(approved.join("nested")).unwrap();
        fs::write(approved.join("nested/items.jsonl"), "safe").unwrap();

        let roots = ApprovedFileRoots::preopen(std::slice::from_ref(&approved)).unwrap();

        assert_eq!(
            read(roots.open(&approved.join("nested/items.jsonl")).unwrap()),
            "safe"
        );
    }

    #[test]
    fn rejects_paths_without_an_approved_root() {
        let workspace = TempDir::new().unwrap();
        let roots = ApprovedFileRoots::preopen(&[]).unwrap();

        assert!(matches!(
            roots.open(&workspace.path().join("items.jsonl")),
            Err(ApprovedFileError::NoApprovedRoots)
        ));
    }

    #[test]
    fn rejects_relative_parent_and_outside_paths() {
        let workspace = TempDir::new().unwrap();
        let approved = workspace.path().join("approved");
        fs::create_dir(&approved).unwrap();
        let roots = ApprovedFileRoots::preopen(std::slice::from_ref(&approved)).unwrap();

        assert!(matches!(
            roots.open(Path::new("items.jsonl")),
            Err(ApprovedFileError::PathNotAbsolute)
        ));
        assert!(matches!(
            roots.open(&approved.join("nested/../items.jsonl")),
            Err(ApprovedFileError::UnsafePath)
        ));
        assert!(matches!(
            roots.open(&workspace.path().join("outside.jsonl")),
            Err(ApprovedFileError::OutsideApprovedRoots)
        ));
    }

    #[test]
    fn rejects_nonregular_files() {
        let workspace = TempDir::new().unwrap();
        let approved = workspace.path().join("approved");
        fs::create_dir_all(approved.join("directory.jsonl")).unwrap();
        let roots = ApprovedFileRoots::preopen(std::slice::from_ref(&approved)).unwrap();

        assert!(matches!(
            roots.open(&approved.join("directory.jsonl")),
            Err(ApprovedFileError::NotRegularFile)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_in_roots_directories_and_final_files() {
        use std::os::unix::fs::symlink;

        let workspace = TempDir::new().unwrap();
        let actual_root = workspace.path().join("actual");
        let linked_root = workspace.path().join("linked");
        let outside = workspace.path().join("outside");
        fs::create_dir_all(actual_root.join("nested")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("items.jsonl"), "outside").unwrap();
        symlink(&actual_root, &linked_root).unwrap();

        assert!(ApprovedFileRoots::preopen(&[linked_root]).is_err());

        let roots = ApprovedFileRoots::preopen(std::slice::from_ref(&actual_root)).unwrap();
        symlink(&outside, actual_root.join("linked-directory")).unwrap();
        symlink(
            outside.join("items.jsonl"),
            actual_root.join("linked-file.jsonl"),
        )
        .unwrap();

        assert!(matches!(
            roots.open(&actual_root.join("linked-directory/items.jsonl")),
            Err(ApprovedFileError::OpenFile(_))
        ));
        assert!(matches!(
            roots.open(&actual_root.join("linked-file.jsonl")),
            Err(ApprovedFileError::OpenFile(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn root_replacement_cannot_redirect_an_open() {
        let workspace = TempDir::new().unwrap();
        let approved = workspace.path().join("approved");
        let held = workspace.path().join("held");
        fs::create_dir(&approved).unwrap();
        fs::write(approved.join("items.jsonl"), "approved").unwrap();
        let roots = ApprovedFileRoots::preopen(std::slice::from_ref(&approved)).unwrap();

        // A canonicalize/check/reopen implementation would now open the new
        // pathname. The held capability must continue to open the approved
        // directory object instead.
        fs::rename(&approved, &held).unwrap();
        fs::create_dir(&approved).unwrap();
        fs::write(approved.join("items.jsonl"), "replacement").unwrap();

        assert_eq!(
            read(roots.open(&approved.join("items.jsonl")).unwrap()),
            "approved"
        );
    }

    #[cfg(unix)]
    #[test]
    fn deepest_nested_root_owns_matching_paths() {
        let workspace = TempDir::new().unwrap();
        let outer = workspace.path().join("outer");
        let nested = outer.join("nested");
        let held_nested = workspace.path().join("held-nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("items.jsonl"), "nested-root").unwrap();
        let roots = ApprovedFileRoots::preopen(&[outer.clone(), nested.clone()]).unwrap();

        fs::rename(&nested, &held_nested).unwrap();
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("items.jsonl"), "outer-root").unwrap();

        assert_eq!(
            read(roots.open(&nested.join("items.jsonl")).unwrap()),
            "nested-root"
        );
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_junction_traversal() {
        use std::process::Command;

        let workspace = TempDir::new().unwrap();
        let approved = workspace.path().join("approved");
        let outside = workspace.path().join("outside");
        fs::create_dir(&approved).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("items.jsonl"), "outside").unwrap();
        let junction = approved.join("junction");
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .status()
            .unwrap();
        assert!(status.success(), "failed to create test junction");

        let roots = ApprovedFileRoots::preopen(std::slice::from_ref(&approved)).unwrap();

        assert!(matches!(
            roots.open(&junction.join("items.jsonl")),
            Err(ApprovedFileError::OpenFile(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn held_windows_root_prevents_path_replacement() {
        let workspace = TempDir::new().unwrap();
        let approved = workspace.path().join("approved");
        fs::create_dir(&approved).unwrap();
        let _roots = ApprovedFileRoots::preopen(std::slice::from_ref(&approved)).unwrap();

        assert!(fs::rename(&approved, workspace.path().join("replacement")).is_err());
    }
}
