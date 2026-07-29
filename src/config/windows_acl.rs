//! Windows owner-only ACL enforcement for `credentials.toml`.
//!
//! Contract (the Windows analogue of the Unix 0600 rule):
//! - the DACL must be present and protected (no inherited ACEs),
//! - it must contain exactly one ACCESS_ALLOWED ACE naming the current
//!   process-token user with exactly the mask this module establishes,
//! - every query, structural check, and API call fails closed.
//!
//! This module never handles credential material; only SIDs, access masks,
//! and descriptor control bits.

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_REVISION, AddAccessAllowedAce,
    DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetLengthSid, GetSecurityDescriptorControl,
    GetSecurityDescriptorDacl, GetTokenInformation, InitializeAcl, IsValidAcl,
    IsValidSecurityDescriptor, IsValidSid, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PRESENT, SE_DACL_PROTECTED, TOKEN_READ, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_ACCESS_RIGHTS, FILE_GENERIC_READ, FILE_GENERIC_WRITE, GetFinalPathNameByHandleW,
    READ_CONTROL, WRITE_DAC,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::ConfigError;

/// The exact rights the owner ACE grants: read/write/delete the file plus
/// READ_CONTROL/WRITE_DAC so this module can re-verify and re-apply the ACL
/// on rewrite. Anything broader (e.g. GENERIC_ALL, FILE_EXECUTE, W write for
/// other trustees) is treated as non-owner access and fails closed.
const OWNER_MASK: FILE_ACCESS_RIGHTS =
    FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE | READ_CONTROL | WRITE_DAC;

/// Applies the owner-only protected DACL to `path` and verifies it stuck.
pub fn set_owner_only_acl(path: &Path) -> Result<(), ConfigError> {
    let user = CurrentUserSid::resolve()?;
    let acl = OwnerOnlyAcl::build(user.as_psid())?;
    let wide = to_wide(path);

    // SAFETY: `wide` is a live NUL-terminated UTF-16 buffer. `acl.ptr()` points
    // to a valid DWORD-aligned ACL buffer owned by `acl`, which outlives the
    // call. Null owner/group/SACL vector elements leave those parts untouched.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl.ptr(),
            ptr::null(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(ConfigError::RestrictionFailed);
    }
    verify_owner_only_acl(path)
}

/// Resolves the target path from the open temp-file handle and applies the
/// owner-only ACL before any content is durably synced.
///
/// Note (residual risk, documented under review): on Windows there is no
/// std-library way to set the security descriptor atomically at `CreateFile`
/// time, so the temp file exists briefly with its inherited ACL before this
/// call tightens it. Because content is written only after this restrict
/// step, and the file lives in a user-private configuration directory, the
/// exposure window contains no credential material. A handle-bound
/// `SetSecurityInfo` apply was considered but rejected: it failed empirically
/// on the native `windows-latest` gate (RestrictionFailed on every store), so
/// the proven by-name path is retained.
pub fn restrict_handle_permissions(file: &File) -> Result<(), ConfigError> {
    use std::os::windows::io::AsRawHandle;
    let path = path_from_handle(file.as_raw_handle() as HANDLE)?;
    set_owner_only_acl(&path)
}

/// Fails closed unless `path` carries exactly the owner-only protected DACL
/// this module establishes.
pub fn enforce_owner_only(path: &Path) -> Result<(), ConfigError> {
    verify_owner_only_acl(path)
}

fn verify_owner_only_acl(path: &Path) -> Result<(), ConfigError> {
    let user = CurrentUserSid::resolve()?;
    let descriptor = NamedSecurityDescriptor::query(path)?;

    if !descriptor.dacl_present_and_protected()? {
        return Err(ConfigError::InsecurePermissions);
    }
    let acl_ptr = descriptor.dacl()?;
    if acl_ptr.is_null() {
        return Err(ConfigError::InsecurePermissions);
    }

    // SAFETY: `acl_ptr` is borrowed from a live security descriptor owned by
    // `descriptor`, which outlives every read below. Structural validity is
    // established before dereferencing more than the fixed ACL header.
    if unsafe { IsValidAcl(acl_ptr) } == 0 {
        return Err(ConfigError::InsecurePermissions);
    }
    // SAFETY: validity established above; reading the fixed-size ACL header is
    // in bounds for a valid ACL.
    let ace_count = unsafe { (*acl_ptr).AceCount };
    if ace_count != 1 {
        return Err(ConfigError::InsecurePermissions);
    }

    let mut ace_raw: *mut core::ffi::c_void = ptr::null_mut();
    // SAFETY: `acl_ptr` is a valid ACL with AceCount == 1, so index 0 exists.
    if unsafe { GetAce(acl_ptr, 0, &mut ace_raw) } == 0 || ace_raw.is_null() {
        return Err(ConfigError::InsecurePermissions);
    }

    // SAFETY: `ace_raw` points into the ACL validated above. The header type
    // is checked before the wider ACCESS_ALLOWED_ACE layout is assumed.
    let header = unsafe { &*(ace_raw.cast::<ACE_HEADER>()) };
    if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE {
        return Err(ConfigError::InsecurePermissions);
    }
    // SAFETY: type checked above; layout matches the Win32 struct.
    let ace = unsafe { &*(ace_raw.cast::<ACCESS_ALLOWED_ACE>()) };
    if ace.Mask != OWNER_MASK {
        return Err(ConfigError::InsecurePermissions);
    }
    // SAFETY: `SidStart` is the documented in-place SID offset of this ACE,
    // within the validated ACL allocation.
    let ace_sid: PSID = ptr::addr_of!(ace.SidStart).cast_mut().cast();
    // SAFETY: both SIDs are live, valid pointers for the duration of the call.
    if unsafe { IsValidSid(ace_sid) } == 0 || unsafe { EqualSid(ace_sid, user.as_psid()) } == 0 {
        return Err(ConfigError::InsecurePermissions);
    }
    Ok(())
}

/// A heap ACL holding exactly one ACCESS_ALLOWED ACE for the current user,
/// backed by `Vec<u32>` so the ACL pointer is always DWORD-aligned.
struct OwnerOnlyAcl(Vec<u32>);

impl OwnerOnlyAcl {
    fn build(user_sid: PSID) -> Result<Self, ConfigError> {
        // SAFETY: `user_sid` is a valid SID for the call.
        let sid_length = unsafe { GetLengthSid(user_sid) } as usize;
        let acl_length =
            size_of::<ACL>() + (size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>()) + sid_length;
        // An ACL must be DWORD-aligned; `Vec<u8>` only guarantees alignment 1,
        // so casting from it to `*mut ACL` is undefined behavior on Win32.
        // Backing the buffer with `Vec<u32>` guarantees the required 4-byte
        // alignment while `InitializeAcl` still receives the exact byte length.
        let mut bytes = vec![0u32; acl_length.div_ceil(size_of::<u32>())];
        let acl_ptr = bytes.as_mut_ptr().cast::<ACL>();

        // SAFETY: `bytes` is DWORD-aligned and holds at least `acl_length`
        // bytes that outlive both calls; the layout matches the documented
        // single-ACE sizing formula.
        if unsafe { InitializeAcl(acl_ptr, acl_length as u32, ACL_REVISION) } == 0 {
            return Err(ConfigError::RestrictionFailed);
        }
        // No inheritance flags: plain allow ACE. Files use ACL_REVISION, not
        // the directory-service revision.
        // SAFETY: buffer sized by the formula above and 4-byte aligned;
        // `user_sid` is valid.
        if unsafe { AddAccessAllowedAce(acl_ptr, ACL_REVISION, OWNER_MASK, user_sid) } == 0 {
            return Err(ConfigError::RestrictionFailed);
        }
        Ok(Self(bytes))
    }

    fn ptr(&self) -> *const ACL {
        self.0.as_ptr().cast()
    }
}

/// RAII owner of a `GetNamedSecurityInfoW` result (freed with `LocalFree`).
struct NamedSecurityDescriptor {
    raw: PSECURITY_DESCRIPTOR,
}

impl NamedSecurityDescriptor {
    fn query(path: &Path) -> Result<Self, ConfigError> {
        let wide = to_wide(path);
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        let mut dacl: *mut ACL = ptr::null_mut();

        // SAFETY: `wide` is a live NUL-terminated UTF-16 buffer. On success the
        // descriptor is owned by us and freed once in Drop; on failure it is
        // freed here if non-null.
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
        if status != ERROR_SUCCESS || descriptor.is_null() {
            if !descriptor.is_null() {
                // SAFETY: a live GetNamedSecurityInfoW result owned by us.
                unsafe { LocalFree(descriptor as HLOCAL) };
            }
            // The ACL could not even be queried: treat as failure to enforce,
            // not as an attacker-modified state.
            return Err(ConfigError::RestrictionFailed);
        }
        // SAFETY: descriptor came from a successful query; validity checked
        // before any field is read.
        if unsafe { IsValidSecurityDescriptor(descriptor) } == 0 {
            // SAFETY: descriptor is owned by us and live.
            unsafe { LocalFree(descriptor as HLOCAL) };
            return Err(ConfigError::InsecurePermissions);
        }
        Ok(Self { raw: descriptor })
    }

    fn dacl_present_and_protected(&self) -> Result<bool, ConfigError> {
        let mut control: u16 = 0;
        let mut revision: u32 = 0;
        // SAFETY: `self.raw` is a valid security descriptor for `self`.
        let ok = unsafe { GetSecurityDescriptorControl(self.raw, &mut control, &mut revision) };
        if ok == 0 {
            return Err(ConfigError::InsecurePermissions);
        }
        Ok(control & SE_DACL_PRESENT != 0 && control & SE_DACL_PROTECTED != 0)
    }

    fn dacl(&self) -> Result<*mut ACL, ConfigError> {
        let mut present: i32 = 0;
        let mut defaulted: i32 = 0;
        let mut dacl: *mut ACL = ptr::null_mut();
        // SAFETY: `self.raw` is valid; the returned ACL pointer is borrowed
        // from the descriptor, not owned.
        let ok =
            unsafe { GetSecurityDescriptorDacl(self.raw, &mut present, &mut dacl, &mut defaulted) };
        if ok == 0 || present == 0 {
            return Err(ConfigError::InsecurePermissions);
        }
        Ok(dacl)
    }
}

impl Drop for NamedSecurityDescriptor {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: `self.raw` is a live GetNamedSecurityInfoW result owned
            // by `self`; freed exactly once here.
            unsafe { LocalFree(self.raw as HLOCAL) };
        }
    }
}

/// RAII owner of the current process token's user SID.
struct CurrentUserSid {
    sid_bytes: Vec<u8>,
}

impl CurrentUserSid {
    fn resolve() -> Result<Self, ConfigError> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: `GetCurrentProcess()` is a valid pseudo-handle; `token` is
        // written only on success and wrapped in `TokenHandle` immediately.
        let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_READ, &mut token) };
        if opened == 0 || token.is_null() {
            return Err(ConfigError::RestrictionFailed);
        }
        let _token = TokenHandle(token);

        let mut needed: u32 = 0;
        // Documented sizing call: fails with ERROR_INSUFFICIENT_BUFFER and
        // reports the required size; we ignore the boolean.
        // SAFETY: `_token` is a valid token handle; null buffer is allowed for
        // the sizing pass. `_token` outlives the call.
        let _ = unsafe {
            GetTokenInformation(_token.raw(), TokenUser, ptr::null_mut(), 0, &mut needed)
        };
        if !(size_of::<TOKEN_USER>()..=4096).contains(&(needed as usize)) {
            return Err(ConfigError::RestrictionFailed);
        }
        // TOKEN_USER embeds a pointer and must be pointer-aligned; a `Vec<u8>`
        // only guarantees alignment 1, so the cast into `*mut TOKEN_USER` would
        // be unaligned. A `Vec<usize>` backing guarantees pointer alignment.
        let word_count = (needed as usize).div_ceil(size_of::<usize>());
        let mut buffer = vec![0usize; word_count];
        // SAFETY: `buffer` holds at least `needed` bytes at pointer alignment
        // and outlives the call; `_token` is valid.
        let ok = unsafe {
            GetTokenInformation(
                _token.raw(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(ConfigError::RestrictionFailed);
        }

        // SAFETY: the API just populated a TOKEN_USER inside `buffer`.
        let sid = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() }.User.Sid;
        if sid.is_null() {
            return Err(ConfigError::RestrictionFailed);
        }
        // SAFETY: `sid` points into `buffer`, which is live; validity is
        // checked before copying exactly the SID's own length.
        if unsafe { IsValidSid(sid) } == 0 {
            return Err(ConfigError::RestrictionFailed);
        }
        // SAFETY: `sid` is valid; copying its self-reported length stays within
        // the source allocation.
        let sid_length = unsafe { GetLengthSid(sid) } as usize;
        let mut sid_bytes = vec![0u8; sid_length];
        // SAFETY: source and destination are live, disjoint, and `sid_length`
        // bytes each.
        unsafe { ptr::copy_nonoverlapping(sid.cast::<u8>(), sid_bytes.as_mut_ptr(), sid_length) };
        Ok(Self { sid_bytes })
    }

    fn as_psid(&self) -> PSID {
        self.sid_bytes.as_ptr().cast_mut().cast()
    }
}

/// Absolute on-disk path for the temp-file handle, with the `\\?\` /
/// `\\?\UNC\` prefixes removed so `SetNamedSecurityInfoW` accepts it.
fn path_from_handle(raw: HANDLE) -> Result<PathBuf, ConfigError> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 1024];
    // SAFETY: `raw` is a live handle borrowed from the caller; `buffer` is
    // sized for the call and truncated to the API-reported length on success.
    let length =
        unsafe { GetFinalPathNameByHandleW(raw, buffer.as_mut_ptr(), buffer.len() as u32, 0) };
    if length == 0 || (length as usize) >= buffer.len() {
        return Err(ConfigError::RestrictionFailed);
    }
    buffer.truncate(length as usize);
    let text = OsString::from_wide(&buffer);
    let text = text.to_string_lossy();
    let plain = match text.strip_prefix("\\\\?\\UNC\\") {
        Some(rest) => format!("\\\\{rest}"),
        None => text.strip_prefix("\\\\?\\").unwrap_or(&text).to_owned(),
    };
    Ok(PathBuf::from(plain))
}

/// RAII wrapper closing a Win32 handle exactly once.
struct TokenHandle(HANDLE);

impl TokenHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for TokenHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` is a valid handle we own; closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
