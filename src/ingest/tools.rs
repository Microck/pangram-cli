//! PATH lookup and subprocess execution for ingest tools.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::output::{CanonicalError, ErrorCode};

use super::usage_with_recovery;

pub(super) fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path_var) {
        let candidate = directory.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = directory.join(format!("{name}.exe"));
            if is_executable(&exe) {
                return Some(exe);
            }
            let cmd = directory.join(format!("{name}.cmd"));
            if is_executable(&cmd) {
                return Some(cmd);
            }
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
}

pub(super) fn require_on_path(name: &str, install: &str) -> Result<PathBuf, CanonicalError> {
    find_on_path(name).ok_or_else(|| {
        usage_with_recovery(
            ErrorCode::MissingDependency,
            &format!("{name} is not installed."),
            install,
        )
    })
}

pub(super) fn run(program: &Path, args: &[&OsStr]) -> Result<String, CanonicalError> {
    let output = Command::new(program).args(args).output().map_err(|_| {
        usage_with_recovery(
            ErrorCode::MissingDependency,
            &format!("could not start {}.", display_name(program)),
            "Install the tool and ensure it is executable.",
        )
    })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let tool = display_name(program);
    Err(usage_with_recovery(
        ErrorCode::TranscriptionFailed,
        &format!("{tool} failed."),
        "Check the local tool installation and retry.",
    ))
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("tool")
        .to_owned()
}
