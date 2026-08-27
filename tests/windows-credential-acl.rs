//! Windows-only integration tests for the owner-only ACL contract on
//! `credentials.toml`. These exercise the real Win32 security APIs against
//! real files in a temp directory; they never touch process-global state and
//! never render credential material into errors.

#![cfg(windows)]

use std::fs;
use std::path::Path;
use std::ptr;

use microck_pangram_cli::config::{ConfigError, CredentialService};
use tempfile::TempDir;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, GetLastError, HLOCAL, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_REVISION, AddAccessAllowedAceEx,
    AllocateAndInitializeSid, DACL_SECURITY_INFORMATION, FreeSid, GetAce, GetLengthSid,
    GetSecurityDescriptorControl, InitializeAcl, IsValidAcl, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PRESENT, SE_DACL_PROTECTED, SECURITY_WORLD_SID_AUTHORITY,
    UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows_sys::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, SECURITY_WORLD_RID};

const SYNTHETIC_KEY: &str = "pangram_synthetic_windows_acl_test_key_0123456789_NOT_A_REAL_KEY";

fn service(root: &TempDir) -> CredentialService {
    CredentialService::new(root.path().join("credentials.toml"))
}

fn to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Queries the DACL of `path` and returns the raw descriptor/ACL pair. The
/// returned `DescriptorGuard` frees the descriptor on drop.
struct DaclInspection {
    _guard: DescriptorGuard,
    acl_ptr: *mut ACL,
}

impl DaclInspection {
    fn ace_count(&self) -> u16 {
        assert!(!self.acl_ptr.is_null(), "DACL must be present");
        // SAFETY: descriptor lives in `_guard`; structural validity checked.
        assert_ne!(unsafe { IsValidAcl(self.acl_ptr) }, 0, "DACL invalid");
        // SAFETY: validity established above; fixed header read is in bounds.
        unsafe { (*self.acl_ptr).AceCount }
    }
}

struct DescriptorGuard(PSECURITY_DESCRIPTOR);

impl Drop for DescriptorGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: live GetNamedSecurityInfoW result owned by us; freed
            // exactly once here.
            unsafe { LocalFree(self.0 as HLOCAL) };
        }
    }
}

/// RAII owner of an `AllocateAndInitializeSid` result (freed with `FreeSid`).
struct WorldSidGuard(PSID);

impl Drop for WorldSidGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` is a live allocated SID owned by us; freed
            // exactly once here.
            unsafe { FreeSid(self.0) };
        }
    }
}

/// Reads descriptor control bits for the inspection backing this file.
fn control_of(path: &Path) -> (u16, DaclInspection) {
    let wide = to_wide(path);
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    // SAFETY: `wide` is a live NUL-terminated UTF-16 buffer; returned
    // descriptor moved into the inspection's guard.
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
    assert_eq!(status, ERROR_SUCCESS);
    let inspection = DaclInspection {
        _guard: DescriptorGuard(descriptor),
        acl_ptr: dacl,
    };
    let mut control: u16 = 0;
    let mut revision: u32 = 0;
    // SAFETY: descriptor live inside the inspection's guard.
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
        0
    );
    (control, inspection)
}

#[test]
fn store_establishes_exact_single_owner_protected_dacl() {
    let root = tempfile::tempdir().unwrap();
    let service = service(&root);
    service.store(SYNTHETIC_KEY).unwrap();

    let (control, inspection) = control_of(service.path());
    assert_ne!(control & SE_DACL_PRESENT, 0, "DACL must be marked present");
    assert_ne!(
        control & SE_DACL_PROTECTED,
        0,
        "DACL must be protected from inheritance"
    );
    assert_eq!(inspection.ace_count(), 1, "exactly one ACE is the contract");

    let mut ace_raw: *mut core::ffi::c_void = ptr::null_mut();
    // SAFETY: valid ACL with AceCount >= 1 read above.
    assert_ne!(
        unsafe { GetAce(inspection.acl_ptr, 0, &mut ace_raw) },
        0,
        "GetAce failed"
    );
    assert!(!ace_raw.is_null());
    // SAFETY: `ace_raw` points into the inspected ACL; header read first.
    let header = unsafe { &*(ace_raw.cast::<ACE_HEADER>()) };
    assert_eq!(
        u32::from(header.AceType),
        ACCESS_ALLOWED_ACE_TYPE,
        "the single ACE must be an allow ACE"
    );
    // The ACE must not be inherited (store writes it fresh, non-inherited).
    const INHERITANCE_FLAGS: u8 = 0x01 | 0x02 | 0x10; // OBJECT_INHERIT | CONTAINER_INHERIT | INHERITED_ACE
    assert_eq!(
        header.AceFlags & INHERITANCE_FLAGS,
        0,
        "the ACE must not be inherited or inheritable: {:#04x}",
        header.AceFlags
    );
    // The strictest condition the verifier enforces: the access mask must be
    // exactly the owner mask (`FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE
    // | READ_CONTROL | WRITE_DAC`). If `SetNamedSecurityInfoW` ever
    // generic-maps the composite bits, `store` would write a file its own
    // `read` rejects; asserting the exact mask makes that regression visible.
    let expected_mask = {
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, READ_CONTROL, WRITE_DAC,
        };
        FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE | READ_CONTROL | WRITE_DAC
    };
    // SAFETY: type checked above; the wide ACCESS_ALLOWED_ACE layout is valid.
    let ace = unsafe { &*(ace_raw.cast::<ACCESS_ALLOWED_ACE>()) };
    assert_eq!(
        ace.Mask, expected_mask,
        "the single ACE must carry exactly the owner mask: stored {:#010x}, expected {expected_mask:#010x}",
        ace.Mask
    );
}

#[test]
fn tampered_extra_trustee_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let service = service(&root);
    service.store(SYNTHETIC_KEY).unwrap();

    add_world_allow_ace(service.path());

    let error = service.read().unwrap_err();
    assert!(
        matches!(error, ConfigError::InsecurePermissions),
        "extra trustee must fail closed: {error}"
    );
}

#[test]
fn tampered_unprotected_inheritance_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let service = service(&root);
    service.store(SYNTHETIC_KEY).unwrap();

    clear_protected_bit(service.path());

    let error = service.read().unwrap_err();
    assert!(
        matches!(error, ConfigError::InsecurePermissions),
        "unprotected DACL must fail closed: {error}"
    );
}

#[test]
fn tamper_does_not_leak_key_into_error() {
    let root = tempfile::tempdir().unwrap();
    let service = service(&root);
    service.store(SYNTHETIC_KEY).unwrap();
    add_world_allow_ace(service.path());

    let error = service.read().unwrap_err();
    let rendered = format!("{error:?} {error}");
    assert!(
        !rendered.contains(SYNTHETIC_KEY),
        "key leaked into error: {rendered}"
    );
}

#[test]
fn rewrite_leaves_no_temp_file_and_acl_roundtrips() {
    let root = tempfile::tempdir().unwrap();
    let service = service(&root);
    service.store(SYNTHETIC_KEY).unwrap();
    service
        .store("pangram_second_windows_acl_synthetic_key_NOT_REAL")
        .unwrap();

    let leftovers: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");

    let resolution = service.read().unwrap();
    assert!(resolution.is_some(), "rewrite must still read back");
}

/// Adds a second ACCESS_ALLOWED ACE for the well-known Everyone SID, keeping
/// the original owner ACE. Used to prove the exact-single-ACE check fires.
fn add_world_allow_ace(path: &Path) {
    // Protected mode retained: only the ACE count changes, so the
    // exact-one-ACE check is what must fire.
    rewrite_dacl_appending_world_ace(
        path,
        0, // world ACE_FLAGS: plain allow, non-inherited
        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
    );
}

/// Rebuilds `path`'s DACL as the existing owner ACE plus an appended world
/// ACE, then applies it under the given security-info flags.
///
/// One shared body drives both tamper scenarios; they differ only in the
/// world ACE's flags and whether protection is retained or explicitly
/// dropped. Keeping the alignment-sensitive unsafe sequence in one place is
/// what the maintainability review requires: a fix to the sizing, alignment,
/// or append logic needs to happen exactly once.
fn rewrite_dacl_appending_world_ace(
    path: &Path,
    world_flags: u32,
    security_info_flags: windows_sys::Win32::Security::OBJECT_SECURITY_INFORMATION,
) {
    let mut world_sid: PSID = ptr::null_mut();
    let authority = SECURITY_WORLD_SID_AUTHORITY;
    // SAFETY: well-known constant authority; `world_sid` written on success.
    let ok = unsafe {
        AllocateAndInitializeSid(
            &authority,
            1,
            SECURITY_WORLD_RID as u32,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            &mut world_sid,
        )
    };
    assert_ne!(ok, 0, "AllocateAndInitializeSid failed");
    assert!(!world_sid.is_null());
    let _world = WorldSidGuard(world_sid);

    // Fetch existing DACL.
    let wide = to_wide(path);
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let mut old_dacl: *mut ACL = ptr::null_mut();
    // SAFETY: `wide` is live; descriptor moved into the guard immediately.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut old_dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(status, ERROR_SUCCESS);
    let _guard = DescriptorGuard(descriptor);
    assert!(!old_dacl.is_null());

    // Read the existing single owner ACE via GetAce so the same owner SID is
    // re-added without introspecting the descriptor layout. `store` wrote
    // exactly one ACCESS_ALLOWED ACE at index 0.
    let mut old_ace_raw: *mut core::ffi::c_void = ptr::null_mut();
    // SAFETY: `old_dacl` is a valid ACL with AceCount >= 1 inside `_guard`.
    assert_ne!(
        unsafe { GetAce(old_dacl, 0, &mut old_ace_raw) },
        0,
        "GetAce failed (err {})",
        unsafe { GetLastError() }
    );
    assert!(!old_ace_raw.is_null());
    // SAFETY: `old_ace_raw` points into `old_dacl` at the ACCESS_ALLOWED_ACE
    // layout that `store` wrote.
    let old_ace = unsafe { &*(old_ace_raw.cast::<ACCESS_ALLOWED_ACE>()) };
    let world_sid_len = unsafe { GetLengthSid(world_sid) } as usize;
    let world_ace_len = (size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>()) + world_sid_len;
    // Sizing upper bound: the whole old ACL plus room for the appended world
    // ACE. InitializeAcl only requires the buffer to be large enough; Add*
    // recomputes each appended ACE's real size from its SID.
    // SAFETY: `old_dacl` valid inside guard; AclSize read is in bounds.
    let new_size = unsafe { (*old_dacl).AclSize } as usize + world_ace_len;

    // Build the two-ACE DACL fresh with InitializeAcl, then append the owner
    // ACE (same SID as production wrote) and the world ACE. memcpy of an
    // existing ACL desynchronizes `AclSize`/`AclFreeSpace`/`AceCount`, which
    // makes any subsequent Add* call fail on the native runner; building the
    // ACL with InitializeAcl + Add* keeps those header fields authoritative.
    //
    // The backing buffer is `Vec<u32>`, not `Vec<u8>`: an ACL must be
    // DWORD-aligned on Win32, and a `Vec<u8>` only guarantees alignment 1, so
    // casting from it to `*mut ACL` would be undefined behavior.
    let mut new_acl = vec![0u32; new_size.div_ceil(size_of::<u32>())];
    let new_acl_ptr = new_acl.as_mut_ptr().cast::<ACL>();
    // SAFETY: `new_acl` is DWORD-aligned, holds at least `new_size` bytes, and
    // outlives all calls.
    assert_ne!(
        unsafe { InitializeAcl(new_acl_ptr, new_size as u32, ACL_REVISION) },
        0,
        "InitializeAcl failed (err {})",
        unsafe { GetLastError() }
    );
    let owner_mask = old_ace.Mask;
    // SAFETY: `old_ace` points into `_guard`, which outlives the call; the
    // in-place SidStart offset is the documented SID location for this ACE.
    let owner_sid = ptr::addr_of!(old_ace.SidStart) as PSID;
    assert_ne!(
        unsafe { AddAccessAllowedAceEx(new_acl_ptr, ACL_REVISION, 0, owner_mask, owner_sid) },
        0,
        "owner AddAccessAllowedAceEx failed (err {})",
        unsafe { GetLastError() }
    );

    let world_mask = {
        use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_GENERIC_WRITE};
        FILE_GENERIC_READ | FILE_GENERIC_WRITE
    };
    // SAFETY: `new_acl` is sized and aligned for both ACEs; `world_sid` is
    // live in `_world`. `world_flags` selects plain allow (0) or an
    // explicitly inherited ACE for the tamper under test.
    assert_ne!(
        unsafe {
            AddAccessAllowedAceEx(
                new_acl_ptr,
                ACL_REVISION,
                world_flags,
                world_mask,
                world_sid,
            )
        },
        0,
        "world AddAccessAllowedAceEx failed (err {})",
        unsafe { GetLastError() }
    );

    // SAFETY: `wide` and `new_acl` are live for the call.
    let apply = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            security_info_flags,
            ptr::null_mut(),
            ptr::null_mut(),
            new_acl_ptr as *const ACL,
            ptr::null(),
        )
    };
    assert_eq!(apply, ERROR_SUCCESS, "tamper SetNamedSecurityInfoW failed");
}

/// Tamper helper: make the DACL unprotected *and* carrying an explicitly
/// inherited ACE, regardless of the runner's directory inheritance policy.
///
/// The contract under test is that `read()` fails closed when the DACL is not
/// an exact owner-only protected DACL. The original implementation only
/// cleared `PROTECTED_DACL_SECURITY_INFORMATION` and assumed the tempdir's
/// ancestry would then flow inherited ACEs in. On GitHub `windows-2025`
/// runners the temp directory carries no inheritable ACEs, so the post-state
/// still read as an exact owner-only DACL with just the protected bit cleared,
/// which is a *different* state than the one the test intends to build.
///
/// To make "unprotected + inherited ACEs" deterministically true we append an
/// explicitly-`INHERITED_ACE`-flagged world ACE with an owner-only DACL, then
/// re-apply it unprotected. The post-state ACE count (>1) and control bits are
/// asserted so a silently-no-op environment fails the test loudly instead of
/// producing a false positive.
fn clear_protected_bit(path: &Path) {
    const INHERITED_ACE_FLAG: u32 = 0x10; // INHERITED_ACE

    // Append an explicitly-inherited world ACE and explicitly drop the DACL
    // protection: the two-ACE DACL is rebuilt via one shared, alignment-safe
    // helper, then re-applied under DACL | UNPROTECTED. Passing
    // UNPROTECTED_DACL_SECURITY_INFORMATION is required because `store`'s DACL
    // is protected; merely omitting PROTECTED_ does not clear the bit.
    rewrite_dacl_appending_world_ace(
        path,
        INHERITED_ACE_FLAG,
        DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
    );

    // Post-condition: the DACL really is unprotected and now carries >1 ACE.
    // If a runner silently discarded either change, fail loudly here instead of
    // letting `read()` pass and producing a false positive.
    let (control, inspection) = control_of(path);
    assert_ne!(control & SE_DACL_PRESENT, 0, "DACL must remain present");
    assert_eq!(
        control & SE_DACL_PROTECTED,
        0,
        "tamper must leave the DACL unprotected"
    );
    assert!(
        inspection.ace_count() > 1,
        "tamper must leave inherited ACEs behind: {}",
        inspection.ace_count()
    );
}
