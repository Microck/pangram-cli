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
    let error = usage_with_recovery(
        ErrorCode::TranscriptionFailed,
        &format!("{tool} failed."),
        "Check the local tool installation and retry.",
    );
    // The tool's own last stderr line is the actionable part ("HTTP Error
    // 403", "cannot open shared object file"). Reduce it like any other
    // untrusted text before it enters canonical details.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let Some(last) = stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
    else {
        return Err(error);
    };
    let mut details = std::collections::BTreeMap::new();
    details.insert(
        "tool_stderr".to_owned(),
        serde_json::Value::from(
            crate::output::sanitize_terminal(last)
                .chars()
                .take(200)
                .collect::<String>(),
        ),
    );
    Err(error.with_details(details).unwrap_or_else(|_| {
        usage_with_recovery(
            ErrorCode::TranscriptionFailed,
            &format!("{tool} failed."),
            "Check the local tool installation and retry.",
        )
    }))
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("tool")
        .to_owned()
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn failed_tool_reports_its_last_stderr_line_sanitized() {
        let script = "printf 'warming up\\nERROR: HTTP Error 403: \\033[31mForbidden\\033[0m\\n\\n' >&2; exit 1";
        let error = super::run(
            Path::new("/bin/sh"),
            &[OsStr::new("-c"), OsStr::new(script)],
        )
        .expect_err("the tool failed");
        assert_eq!(error.code(), crate::output::ErrorCode::TranscriptionFailed);
        let details = serde_json::to_value(error.details().expect("details")).unwrap();
        let line = details["tool_stderr"].as_str().expect("tool_stderr");
        assert!(line.contains("HTTP Error 403"), "{line}");
        assert!(
            !line.contains('\u{1b}'),
            "terminal controls must be stripped: {line:?}"
        );
    }
}
