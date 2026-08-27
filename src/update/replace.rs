//! Direct-install replacement and receipt finalization.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::state::{parse_install_receipt, validate_parsed_install_receipt};
use super::{InstallReceipt, Target, UpdateError, UpdateErrorKind, validate_install_receipt};
use crate::domain::{Sha256Hash, UtcTimestamp};

/// Verified executable bytes and signed release identity for one replacement.
pub struct DirectUpdateCandidate<'a> {
    executable: &'a [u8],
    version: &'a str,
    manifest_sha256: Sha256Hash,
    installed_at: UtcTimestamp,
}

/// Whether replacement and receipt publication finished in this process.
#[derive(Debug)]
pub enum DirectReplacement {
    Completed(InstallReceipt),
    ReplacementStarted(InstallReceipt),
}

enum ReplacementContext {
    RunningExecutable,
    ExternalCandidate,
}

impl<'a> DirectUpdateCandidate<'a> {
    #[must_use]
    pub const fn new(
        executable: &'a [u8],
        version: &'a str,
        manifest_sha256: Sha256Hash,
        installed_at: UtcTimestamp,
    ) -> Self {
        Self {
            executable,
            version,
            manifest_sha256,
            installed_at,
        }
    }
}

/// Replaces one receipt-owned direct installation. Candidate validation and
/// a version smoke test complete before the executable path is mutated.
pub fn replace_direct_install(
    current_executable: &Path,
    receipt_path: &Path,
    current_version: &str,
    target: Target,
    candidate: DirectUpdateCandidate<'_>,
) -> Result<DirectReplacement, UpdateError> {
    let receipt_bytes = read_protected(receipt_path)?;
    validate_install_receipt(&receipt_bytes, current_executable, current_version, target)?;
    let current_metadata = fs::symlink_metadata(current_executable).map_err(|_| replace_error())?;
    if !current_metadata.file_type().is_file() {
        return Err(replace_error());
    }

    replace_owned_direct_install(
        current_executable,
        receipt_path,
        target,
        candidate,
        ReplacementContext::RunningExecutable,
    )
}

/// Installs a verified candidate into an empty destination or replaces an
/// exact receipt-owned direct install. Mixed or unowned filesystem state is
/// rejected without mutation.
pub fn install_direct_candidate(
    destination: &Path,
    receipt_path: &Path,
    target: Target,
    candidate: DirectUpdateCandidate<'_>,
) -> Result<DirectReplacement, UpdateError> {
    let destination_exists = regular_file_exists(destination)?;
    let receipt_exists = regular_file_exists(receipt_path)?;
    match (destination_exists, receipt_exists) {
        (false, false) => install_initial_direct(destination, receipt_path, target, candidate),
        (true, true) => {
            let receipt_bytes = read_protected(receipt_path)?;
            let receipt = parse_install_receipt(&receipt_bytes)?;
            validate_parsed_install_receipt(
                &receipt,
                destination,
                receipt.installed_version(),
                target,
            )?;
            if !smoke_version(destination, receipt.installed_version()) {
                return Err(not_owned_error());
            }
            replace_owned_direct_install(
                destination,
                receipt_path,
                target,
                candidate,
                ReplacementContext::ExternalCandidate,
            )
        }
        _ => Err(not_owned_error()),
    }
}

fn replace_owned_direct_install(
    current_executable: &Path,
    receipt_path: &Path,
    target: Target,
    candidate: DirectUpdateCandidate<'_>,
    context: ReplacementContext,
) -> Result<DirectReplacement, UpdateError> {
    let staged_executable = stage_executable(current_executable, candidate.executable)?;
    if !smoke_version(&staged_executable, candidate.version) {
        let _ = fs::remove_file(&staged_executable);
        return Err(replace_error());
    }

    let new_receipt = InstallReceipt::new(
        current_executable,
        candidate.version,
        target,
        candidate.manifest_sha256,
        candidate.installed_at,
    )?;
    let pending_receipt = pending_receipt_path(receipt_path)?;
    write_pending_receipt(&pending_receipt, &new_receipt)?;

    #[cfg(windows)]
    if matches!(context, ReplacementContext::RunningExecutable) {
        if let Err(error) = spawn_windows_replacer(
            &staged_executable,
            current_executable,
            receipt_path,
            candidate.version,
            target,
        ) {
            let _ = fs::remove_file(&staged_executable);
            let _ = fs::remove_file(&pending_receipt);
            return Err(error);
        }
        return Ok(DirectReplacement::ReplacementStarted(new_receipt));
    }
    #[cfg(unix)]
    let _ = context;

    if let Err(error) = replace_platform(current_executable, &staged_executable) {
        let _ = fs::remove_file(&staged_executable);
        let _ = fs::remove_file(&pending_receipt);
        return Err(error);
    }
    if !smoke_version(current_executable, candidate.version) {
        // The pending protected receipt is deliberate recovery state. A later
        // finalization attempt needs no download or second replacement.
        return Err(replace_error());
    }
    publish_pending_receipt(&pending_receipt, receipt_path)?;
    Ok(DirectReplacement::Completed(new_receipt))
}

fn install_initial_direct(
    destination: &Path,
    receipt_path: &Path,
    target: Target,
    candidate: DirectUpdateCandidate<'_>,
) -> Result<DirectReplacement, UpdateError> {
    // Constructing the receipt validates the destination and version before
    // even parent directories are created. Publication still waits until both
    // candidate and installed-path smoke tests succeed.
    let receipt = InstallReceipt::new(
        destination,
        candidate.version,
        target,
        candidate.manifest_sha256,
        candidate.installed_at,
    )?;
    let destination_parent = destination.parent().ok_or_else(replace_error)?;
    let receipt_parent = receipt_path.parent().ok_or_else(replace_error)?;
    fs::create_dir_all(destination_parent).map_err(|_| replace_error())?;
    fs::create_dir_all(receipt_parent).map_err(|_| replace_error())?;

    let staged_executable = stage_executable(destination, candidate.executable)?;
    if !smoke_version(&staged_executable, candidate.version) {
        let _ = fs::remove_file(&staged_executable);
        return Err(replace_error());
    }
    let pending_receipt = pending_receipt_path(receipt_path)?;
    write_pending_receipt(&pending_receipt, &receipt)?;

    // A hard link publishes the already-synced candidate only if the
    // destination is still absent. This closes the race between the initial
    // ownership check and publication without a platform-specific rename flag.
    if fs::hard_link(&staged_executable, destination).is_err() {
        let _ = fs::remove_file(&staged_executable);
        let _ = fs::remove_file(&pending_receipt);
        return Err(replace_error());
    }
    if fs::remove_file(&staged_executable).is_err() {
        let _ = fs::remove_file(destination);
        let _ = fs::remove_file(&pending_receipt);
        return Err(replace_error());
    }
    if !smoke_version(destination, candidate.version) {
        let _ = fs::remove_file(destination);
        let _ = fs::remove_file(&pending_receipt);
        return Err(replace_error());
    }
    if publish_new_receipt(&pending_receipt, receipt_path).is_err() {
        let _ = fs::remove_file(destination);
        let _ = fs::remove_file(&pending_receipt);
        return Err(replace_error());
    }
    Ok(DirectReplacement::Completed(receipt))
}

fn regular_file_exists(path: &Path) -> Result<bool, UpdateError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(not_owned_error()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(replace_error()),
    }
}

/// Publishes the protected pending receipt left after a successful binary
/// replacement. This path never downloads or replaces the executable again.
pub fn finalize_pending_receipt(
    current_executable: &Path,
    receipt_path: &Path,
    current_version: &str,
    target: Target,
) -> Result<InstallReceipt, UpdateError> {
    let pending = pending_receipt_path(receipt_path)?;
    let pending_bytes = match fs::metadata(&pending) {
        Ok(metadata) if metadata.file_type().is_file() => read_protected(&pending)?,
        Ok(_) => return Err(replace_error()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let current_receipt = read_protected(receipt_path)?;
            return validate_install_receipt(
                &current_receipt,
                current_executable,
                current_version,
                target,
            );
        }
        Err(_) => return Err(replace_error()),
    };
    let receipt =
        validate_install_receipt(&pending_bytes, current_executable, current_version, target)?;
    let metadata = fs::symlink_metadata(current_executable).map_err(|_| replace_error())?;
    if !metadata.file_type().is_file() || !smoke_version(current_executable, current_version) {
        return Err(replace_error());
    }
    publish_pending_receipt(&pending, receipt_path)?;
    Ok(receipt)
}

fn read_protected(path: &Path) -> Result<Vec<u8>, UpdateError> {
    crate::config::enforce_protected_permissions(path).map_err(|_| replace_error())?;
    fs::read(path).map_err(|_| replace_error())
}

fn write_pending_receipt(path: &Path, receipt: &InstallReceipt) -> Result<(), UpdateError> {
    let mut serialized = serde_json::to_vec_pretty(receipt).map_err(|_| replace_error())?;
    serialized.push(b'\n');
    crate::config::atomic_secret_write(path, &serialized).map_err(|_| replace_error())
}

fn stage_executable(destination: &Path, bytes: &[u8]) -> Result<PathBuf, UpdateError> {
    let parent = destination.parent().ok_or_else(replace_error)?;
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(replace_error)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staged = parent.join(format!(".{name}.update-{}-{nonce}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o755);
        }
        let mut file = options.open(&staged)?;
        file.write_all(bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(fs::Permissions::from_mode(0o755))?;
        }
        file.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staged);
        return Err(replace_error());
    }
    Ok(staged)
}

fn smoke_version(executable: &Path, expected_version: &str) -> bool {
    smoke_version_with_retry_observer(executable, expected_version, || {})
}

fn smoke_version_with_retry_observer(
    executable: &Path,
    expected_version: &str,
    mut observe_retry: impl FnMut(),
) -> bool {
    const ATTEMPTS: usize = 5;
    const RETRY_DELAY: Duration = Duration::from_millis(25);

    for attempt in 0..ATTEMPTS {
        let output = Command::new(executable)
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(output) => {
                return output.status.success()
                    && String::from_utf8(output.stdout).is_ok_and(|stdout| {
                        stdout.trim_end() == format!("pangram {expected_version}")
                    });
            }
            Err(error) if attempt + 1 < ATTEMPTS && transient_spawn_error(&error) => {
                observe_retry();
                std::thread::sleep(RETRY_DELAY);
            }
            Err(_) => return false,
        }
    }
    false
}

fn transient_spawn_error(error: &std::io::Error) -> bool {
    let kind = error.kind();
    matches!(
        kind,
        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
    ) || (cfg!(target_os = "linux") && kind == std::io::ErrorKind::ExecutableFileBusy)
}

#[cfg(unix)]
fn replace_platform(destination: &Path, staged: &Path) -> Result<(), UpdateError> {
    fs::rename(staged, destination).map_err(|_| replace_error())?;
    sync_parent_directory(destination);
    Ok(())
}

#[cfg(windows)]
fn replace_platform(destination: &Path, staged: &Path) -> Result<(), UpdateError> {
    replace_file_windows(destination, staged)
}

#[cfg(windows)]
fn spawn_windows_replacer(
    staged: &Path,
    destination: &Path,
    receipt_path: &Path,
    version: &str,
    target: Target,
) -> Result<(), UpdateError> {
    Command::new(staged)
        .args([
            "__pangram-update-replace",
            "--parent-pid",
            &std::process::id().to_string(),
            "--destination",
            destination.to_str().ok_or_else(replace_error)?,
            "--receipt",
            receipt_path.to_str().ok_or_else(replace_error)?,
            "--version",
            version,
            "--target",
            target.as_str(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| replace_error())
}

/// Handles the Windows-only post-parent replacement mode before public CLI
/// parsing. Returns `None` for every normal invocation.
#[cfg(windows)]
pub(crate) fn run_windows_replace_helper<I, T>(arguments: I) -> Option<u8>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString>,
{
    let arguments = arguments
        .into_iter()
        .map(Into::into)
        .collect::<Vec<std::ffi::OsString>>();
    if arguments.get(1).and_then(|value| value.to_str()) != Some("__pangram-update-replace") {
        return None;
    }
    Some(
        if windows_replace(arguments.get(2..).unwrap_or_default()).is_ok() {
            0
        } else {
            1
        },
    )
}

#[cfg(windows)]
fn windows_replace(arguments: &[std::ffi::OsString]) -> Result<(), UpdateError> {
    if arguments.len() != 10 {
        return Err(replace_error());
    }
    let value = |flag: &str, index: usize| -> Result<&str, UpdateError> {
        if arguments.get(index).and_then(|value| value.to_str()) != Some(flag) {
            return Err(replace_error());
        }
        arguments
            .get(index + 1)
            .and_then(|value| value.to_str())
            .ok_or_else(replace_error)
    };
    let parent_pid = value("--parent-pid", 0)?
        .parse::<u32>()
        .map_err(|_| replace_error())?;
    if parent_pid == 0 || parent_pid == std::process::id() {
        return Err(replace_error());
    }
    let destination = PathBuf::from(value("--destination", 2)?);
    let receipt_path = PathBuf::from(value("--receipt", 4)?);
    let version = value("--version", 6)?;
    let target = match value("--target", 8)? {
        "x86_64-pc-windows-msvc" => Target::X86_64PcWindowsMsvc,
        _ => return Err(replace_error()),
    };
    let staged = std::env::current_exe().map_err(|_| replace_error())?;
    if staged.parent() != destination.parent() || !fs::metadata(&staged).is_ok_and(|m| m.is_file())
    {
        return Err(replace_error());
    }
    let pending = pending_receipt_path(&receipt_path)?;
    let pending_bytes = read_protected(&pending)?;
    validate_install_receipt(&pending_bytes, &destination, version, target)?;
    wait_for_parent(parent_pid)?;
    replace_file_windows(&destination, &staged)?;
    if !smoke_version(&destination, version) {
        return Err(replace_error());
    }
    publish_pending_receipt(&pending, &receipt_path)
}

#[cfg(windows)]
fn wait_for_parent(parent_pid: u32) -> Result<(), UpdateError> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_FAILED};
    use windows_sys::Win32::System::Threading::{INFINITE, OpenProcess, WaitForSingleObject};

    // windows-sys does not expose this standard process access right in the
    // Threading module, but OpenProcess accepts its documented u32 value.
    const PROCESS_SYNCHRONIZE: u32 = 0x0010_0000;

    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, parent_pid) };
    if handle.is_null() {
        return Err(replace_error());
    }
    let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
    unsafe { CloseHandle(handle) };
    if wait == WAIT_FAILED {
        return Err(replace_error());
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file_windows(destination: &Path, staged: &Path) -> Result<(), UpdateError> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};

    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let staged = staged
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            staged.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        return Err(replace_error());
    }
    Ok(())
}

fn pending_receipt_path(receipt_path: &Path) -> Result<PathBuf, UpdateError> {
    let parent = receipt_path.parent().ok_or_else(replace_error)?;
    let name = receipt_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(replace_error)?;
    Ok(parent.join(format!(".{name}.pending")))
}

fn publish_pending_receipt(pending: &Path, receipt: &Path) -> Result<(), UpdateError> {
    fs::rename(pending, receipt).map_err(|_| replace_error())?;
    #[cfg(unix)]
    sync_parent_directory(receipt);
    Ok(())
}

fn publish_new_receipt(pending: &Path, receipt: &Path) -> Result<(), UpdateError> {
    fs::hard_link(pending, receipt).map_err(|_| replace_error())?;
    if fs::remove_file(pending).is_err() {
        let _ = fs::remove_file(receipt);
        return Err(replace_error());
    }
    #[cfg(unix)]
    sync_parent_directory(receipt);
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) {
    if let Some(parent) = path.parent()
        && let Ok(directory) = fs::File::open(parent)
    {
        let _ = directory.sync_all();
    }
}

const fn replace_error() -> UpdateError {
    UpdateError::new(
        UpdateErrorKind::ReplaceFailed,
        "The direct installation could not be replaced safely.",
    )
}

const fn not_owned_error() -> UpdateError {
    UpdateError::new(
        UpdateErrorKind::InstallNotOwned,
        "The install destination is not owned by a matching direct-install receipt.",
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use super::{smoke_version, smoke_version_with_retry_observer};

    #[test]
    fn version_smoke_retries_linux_executable_busy() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("pangram");
        fs::write(&executable, b"#!/bin/sh\nprintf 'pangram 1.2.3\\n'\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

        // Linux rejects exec with ETXTBSY while the file is open for writing.
        // Release that real contention only after the failed spawn is observed.
        let mut writer = Some(
            fs::OpenOptions::new()
                .write(true)
                .open(&executable)
                .unwrap(),
        );
        let mut retries = 0;

        assert!(smoke_version_with_retry_observer(
            &executable,
            "1.2.3",
            || {
                retries += 1;
                drop(writer.take());
            }
        ));
        assert_eq!(retries, 1);
    }

    #[test]
    fn version_smoke_does_not_retry_a_started_wrong_version() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("pangram");
        fs::write(
            &executable,
            b"#!/bin/sh\nif [ -e \"$0.ran\" ]; then printf 'pangram 1.2.3\\n'; else : >\"$0.ran\"; printf 'pangram 9.9.9\\n'; fi\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(!smoke_version(&executable, "1.2.3"));
        assert!(executable.with_extension("ran").is_file());
    }

    #[test]
    fn version_smoke_does_not_retry_a_started_nonzero_exit() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("pangram");
        fs::write(
            &executable,
            b"#!/bin/sh\nif [ -e \"$0.ran\" ]; then printf 'pangram 1.2.3\\n'; else : >\"$0.ran\"; exit 9; fi\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(!smoke_version(&executable, "1.2.3"));
        assert!(executable.with_extension("ran").is_file());
    }
}
