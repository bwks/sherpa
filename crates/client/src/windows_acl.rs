use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetTokenInformation, NO_INHERITANCE,
    PROTECTED_DACL_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid token handle returned by `OpenProcessToken`
        // and this type owns it exclusively.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct LocalAcl(*mut ACL);

impl Drop for LocalAcl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` was allocated by `SetEntriesInAclW` and has not
            // been freed elsewhere.
            unsafe {
                let _ = LocalFree(self.0.cast());
            }
        }
    }
}

struct CurrentUserSid {
    storage: Vec<usize>,
}

impl CurrentUserSid {
    fn load() -> Result<Self> {
        let mut token_handle = null_mut();

        // SAFETY: `token_handle` points to writable storage and the process
        // pseudo-handle is valid for the duration of this call.
        let opened =
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle) };
        if opened == 0 {
            return Err(io::Error::last_os_error()).context("Failed to open current process token");
        }
        let token = OwnedHandle(token_handle);

        let mut required_bytes = 0u32;
        // SAFETY: A null buffer with length zero is the documented size-query
        // form of `GetTokenInformation`.
        unsafe {
            let _ = GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required_bytes);
        }
        if required_bytes == 0 {
            return Err(io::Error::last_os_error())
                .context("Failed to determine current user SID size");
        }

        let word_size = size_of::<usize>();
        let word_count = (required_bytes as usize).div_ceil(word_size);
        let mut storage = vec![0usize; word_count];

        // SAFETY: `storage` is aligned for `TOKEN_USER`, has at least
        // `required_bytes` writable bytes, and remains alive in the returned
        // value for as long as its embedded SID is used.
        let loaded = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                storage.as_mut_ptr().cast(),
                required_bytes,
                &mut required_bytes,
            )
        };
        if loaded == 0 {
            return Err(io::Error::last_os_error()).context("Failed to read current user SID");
        }

        Ok(Self { storage })
    }

    fn as_ptr(&self) -> PSID {
        // SAFETY: `storage` was populated by `GetTokenInformation` for the
        // `TokenUser` information class and is aligned for `TOKEN_USER`.
        unsafe {
            let token_user = &*self.storage.as_ptr().cast::<TOKEN_USER>();
            token_user.User.Sid
        }
    }
}

/// Replace a file's DACL with a protected entry for the current user only.
#[tracing::instrument(level = "debug", fields(path = %path.display()))]
pub(crate) fn restrict_file_to_current_user(path: &Path) -> Result<()> {
    let current_user = CurrentUserSid::load()?;
    let user_sid = current_user.as_ptr();
    if user_sid.is_null() {
        bail!("Current process token did not contain a user SID");
    }

    let trustee = TRUSTEE_W {
        pMultipleTrustee: null_mut(),
        MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_USER,
        ptstrName: user_sid.cast(),
    };
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_ALL,
        grfAccessMode: SET_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: trustee,
    };

    let mut acl = null_mut();
    // SAFETY: `entry` and the SID it references remain valid for this call;
    // `acl` points to writable output storage and starts null as required when
    // creating a new ACL.
    let acl_status = unsafe { SetEntriesInAclW(1, &entry, null(), &mut acl) };
    if acl_status != 0 {
        return Err(io::Error::from_raw_os_error(acl_status as i32))
            .context("Failed to create protected SSH private key ACL");
    }
    let acl = LocalAcl(acl);

    let mut wide_path: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide_path.contains(&0) {
        bail!("SSH private key path contains an embedded NUL");
    }
    wide_path.push(0);

    // SAFETY: `wide_path` is NUL-terminated and alive for the call; `acl` is a
    // valid ACL created by `SetEntriesInAclW`. Owner, group, and SACL are null
    // because the flags request only a protected DACL update.
    let set_status = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl.0,
            null(),
        )
    };
    if set_status != 0 {
        return Err(io::Error::from_raw_os_error(set_status as i32))
            .context("Failed to apply protected SSH private key ACL");
    }

    Ok(())
}

#[cfg(test)]
pub(crate) fn is_restricted_to_current_user(path: &Path) -> Result<bool> {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL_SIZE_INFORMATION, AclSizeInformation, EqualSid, GetAce,
        GetAclInformation, GetSecurityDescriptorControl, INHERITED_ACE, PSECURITY_DESCRIPTOR,
        SE_DACL_PROTECTED,
    };

    struct LocalSecurityDescriptor(PSECURITY_DESCRIPTOR);

    impl Drop for LocalSecurityDescriptor {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: `self.0` was allocated by `GetNamedSecurityInfoW`
                // and is owned by this value.
                unsafe {
                    let _ = LocalFree(self.0);
                }
            }
        }
    }

    let current_user = CurrentUserSid::load()?;
    let mut wide_path: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide_path.push(0);

    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: `wide_path` is NUL-terminated and the output pointers refer to
    // writable storage. The returned descriptor is released below.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32))
            .context("Failed to inspect SSH private key ACL");
    }
    let descriptor = LocalSecurityDescriptor(descriptor);
    if dacl.is_null() || descriptor.0.is_null() {
        return Ok(false);
    }

    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: `descriptor.0` is a valid security descriptor returned by
    // `GetNamedSecurityInfoW`; the output pointers are writable.
    let control_loaded =
        unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) };
    if control_loaded == 0 {
        return Err(io::Error::last_os_error())
            .context("Failed to inspect SSH private key ACL protection");
    }
    if control & SE_DACL_PROTECTED == 0 {
        return Ok(false);
    }

    let mut acl_info = ACL_SIZE_INFORMATION::default();
    // SAFETY: `dacl` is valid while `descriptor` is alive and `acl_info`
    // provides correctly sized writable output storage.
    let acl_loaded = unsafe {
        GetAclInformation(
            dacl,
            (&mut acl_info as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if acl_loaded == 0 {
        return Err(io::Error::last_os_error()).context("Failed to inspect SSH private key ACEs");
    }
    if acl_info.AceCount != 1 {
        return Ok(false);
    }

    let mut ace: *mut c_void = null_mut();
    // SAFETY: `dacl` contains one ACE and `ace` points to writable output
    // storage for the borrowed ACE pointer.
    let ace_loaded = unsafe { GetAce(dacl, 0, &mut ace) };
    if ace_loaded == 0 || ace.is_null() {
        return Err(io::Error::last_os_error()).context("Failed to inspect SSH private key ACE");
    }

    // SAFETY: The ACL was built by `SetEntriesInAclW` from one allow entry, so
    // its sole ACE has the `ACCESS_ALLOWED_ACE` layout.
    let allowed_ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
    if allowed_ace.Header.AceFlags as u32 & INHERITED_ACE != 0 {
        return Ok(false);
    }
    let ace_sid = (&raw const allowed_ace.SidStart).cast_mut().cast();

    // SAFETY: Both SID pointers are valid for this call while `descriptor` and
    // `current_user` remain alive.
    Ok(unsafe { EqualSid(ace_sid, current_user.as_ptr()) } != 0)
}
