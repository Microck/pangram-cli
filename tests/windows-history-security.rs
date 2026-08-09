//! Native Windows history filesystem security checks.
//!
//! These tests use real Win32 DACL and reparse-point behavior over a real
//! bundled SQLite database. They are intentionally separate from the
//! credential ACL gate so CI can prove both policies independently.

#![cfg(windows)]

use std::path::Path;
use std::ptr;

use microck_pangram_cli::history::{HistoryErrorCode, HistoryStore};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, IsValidAcl, PSECURITY_DESCRIPTOR,
    SE_DACL_PRESENT, SE_DACL_PROTECTED,
};

fn to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

struct DescriptorGuard(PSECURITY_DESCRIPTOR);

impl Drop for DescriptorGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this is the live descriptor returned by
            // GetNamedSecurityInfoW and is released exactly once.
            unsafe { LocalFree(self.0 as HLOCAL) };
        }
    }
}

fn assert_owner_only(path: &Path) {
    let wide = to_wide(path);
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    // SAFETY: `wide` is NUL terminated and the output pointers remain live
    // until moved into `DescriptorGuard`.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(status, ERROR_SUCCESS, "read DACL for {}", path.display());
    let _guard = DescriptorGuard(descriptor);
    assert!(!dacl.is_null(), "DACL is present for {}", path.display());
    // SAFETY: the descriptor owns `dacl` for the lifetime of `_guard`.
    assert_ne!(unsafe { IsValidAcl(dacl) }, 0, "valid DACL");
    // SAFETY: structural validity was established above.
    assert_eq!(unsafe { (*dacl).AceCount }, 1, "exactly one owner ACE");

    let mut control = 0_u16;
    let mut revision = 0_u32;
    // SAFETY: `descriptor` remains live under `_guard`.
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
        0
    );
    assert_ne!(control & SE_DACL_PRESENT, 0, "DACL marked present");
    assert_ne!(
        control & SE_DACL_PROTECTED,
        0,
        "DACL protected from inheritance"
    );
}

#[test]
fn database_wal_and_shm_have_owner_only_protected_acls() {
    let root = tempfile::tempdir().expect("temporary data root");
    let store = HistoryStore::open(root.path()).expect("open real history");
    let database = store.database_path();
    let history = database.parent().expect("history parent");
    let wal = database.with_extension("db-wal");
    let shm = database.with_extension("db-shm");

    assert!(wal.is_file(), "WAL pin exists while the store is open");
    assert!(shm.is_file(), "SHM pin exists while the store is open");
    for path in [history, database.as_path(), wal.as_path(), shm.as_path()] {
        assert_owner_only(path);
    }
}

#[test]
fn database_and_sidecar_reparse_points_fail_closed_without_target_mutation() {
    use std::os::windows::fs::symlink_file;

    for suffix in ["db", "db-wal", "db-shm"] {
        let root = tempfile::tempdir().expect("temporary data root");
        let initialized = HistoryStore::open(root.path()).expect("initialize history");
        let database = initialized.database_path();
        drop(initialized);

        let hostile = root.path().join(format!("hostile-{suffix}"));
        let sentinel = b"hostile reparse target sentinel";
        std::fs::write(&hostile, sentinel).expect("write target");
        let alias = if suffix == "db" {
            std::fs::remove_file(&database).expect("remove database for alias");
            database
        } else {
            database.with_extension(suffix)
        };
        symlink_file(&hostile, &alias).expect("create file reparse point");

        let error = HistoryStore::open(root.path()).expect_err("reparse point must fail closed");
        assert_eq!(error.code(), HistoryErrorCode::HistoryUnavailable);
        assert_eq!(
            std::fs::read(&hostile).expect("read target"),
            sentinel,
            "rejected open must not mutate target"
        );
        assert!(
            std::fs::symlink_metadata(&alias)
                .expect("alias remains")
                .file_type()
                .is_symlink(),
            "rejected alias remains intact"
        );
    }
}
