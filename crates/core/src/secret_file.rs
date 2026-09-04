//! Restrict a file on disk to its owner.
//!
//! Several files Pylon writes are secrets: the OIDC provider's RSA signing
//! key, the CLI's cloud credentials, the signed-file-URL MAC secret. On unix
//! these are chmod 0600. This module is the same guarantee expressed once,
//! so the Windows build does not quietly ship the weaker default.
//!
//! On Windows the equivalent of 0600 is a DACL holding a single access-allowed
//! entry for the current user, with inheritance from the parent directory
//! switched off. Inheritance is the part that matters: a file created under a
//! directory whose ACL grants `Users` read access — anything under `C:\` that
//! is not inside a user profile, and any mapped share — inherits that grant,
//! so writing the key and stopping there leaves it readable by every account
//! on the machine.

use std::io;
use std::path::Path;

/// Restrict `path` so only the current user can read or write it.
///
/// Returns an error if the permissions could not be applied. Callers holding
/// a secret should treat that as fatal rather than proceeding with a file
/// that may be world-readable.
pub fn restrict_to_owner(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(windows)]
    {
        windows_impl::restrict_to_owner(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "restricting a file to its owner is not implemented on this platform",
        ))
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE,
        SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_GENERIC_READ, FILE_GENERIC_WRITE};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Owner of a `LocalAlloc`'d pointer, so every early return frees it.
    struct LocalPtr(*mut core::ffi::c_void);

    impl Drop for LocalPtr {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer came from an API documented to return
                // memory the caller frees with LocalFree, and is freed once.
                unsafe { LocalFree(self.0) };
            }
        }
    }

    /// Owner of a process token handle.
    struct TokenHandle(HANDLE);

    impl Drop for TokenHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the handle came from OpenProcessToken and is closed once.
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    pub fn restrict_to_owner(path: &Path) -> io::Result<()> {
        // The current user's SID, read out of this process's access token.
        // `TOKEN_USER` is a header pointing into the same allocation, so the
        // buffer has to outlive every use of the SID.
        let token = open_process_token()?;
        let user_buf = token_user(&token)?;
        // SAFETY: `token_user` returned a buffer sized by the API itself and
        // laid out as a TOKEN_USER, and `user_buf` keeps it alive here.
        let sid = unsafe { (*(user_buf.as_ptr() as *const TOKEN_USER)).User.Sid };

        // One access-allowed entry: this user, read + write + delete, not
        // inherited by anything and not inheriting anything.
        let access = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE,
            grfAccessMode: SET_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: sid as *mut u16,
            },
        };

        let mut acl: *mut ACL = ptr::null_mut();
        // SAFETY: one well-formed EXPLICIT_ACCESS_W in, an out-pointer the
        // API allocates. A non-zero return means nothing was allocated.
        let status = unsafe { SetEntriesInAclW(1, &access, ptr::null(), &mut acl) };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let _acl_guard = LocalPtr(acl as *mut core::ffi::c_void);

        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);

        // PROTECTED_DACL_SECURITY_INFORMATION is what drops the inherited
        // entries. Without it the new entry is merely added alongside
        // whatever the parent directory grants.
        //
        // SAFETY: a null-terminated path, an ACL from SetEntriesInAclW, and
        // null for the owner/group/SACL fields the flags do not select.
        let status = unsafe {
            SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                acl,
                ptr::null(),
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }

    fn open_process_token() -> io::Result<TokenHandle> {
        let mut handle: HANDLE = ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo-handle needing no close;
        // `handle` is an out-parameter the call fills on success.
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(TokenHandle(handle))
    }

    /// Read `TokenUser` into a caller-owned buffer. The first call is expected
    /// to fail with the required size; that is the documented protocol, not an
    /// error path.
    fn token_user(token: &TokenHandle) -> io::Result<Vec<u8>> {
        let mut needed: u32 = 0;
        // SAFETY: a null buffer with zero length asks only for the size.
        unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0u8; needed as usize];
        // SAFETY: the buffer is exactly the size the call just asked for.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::restrict_to_owner;

    /// The file stays readable by the process that locked it down. A DACL
    /// that names the wrong trustee would still "succeed" at the API level
    /// and only show up as an unreadable secret later.
    #[test]
    fn owner_can_still_read_the_file() {
        let dir = std::env::temp_dir().join(format!("pylon-secret-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secret");
        std::fs::write(&path, b"shh").unwrap();

        restrict_to_owner(&path).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"shh");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn unix_mode_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pylon-secret-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secret");
        std::fs::write(&path, b"shh").unwrap();

        restrict_to_owner(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0600, got 0o{mode:o}");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
