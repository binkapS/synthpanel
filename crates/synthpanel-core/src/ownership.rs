//! File ownership helpers.
//!
//! The privileged core creates the files the unprivileged web process needs
//! (the IPC token, the secret key, the panel certificate). They have to
//! belong to the web user, otherwise the web service cannot start.

use std::path::Path;

/// The user the web process runs as.
pub const WEB_USER: &str = "synthpanel";

/// Hand a path to the web user.
///
/// This is best-effort: on a development machine the user does not exist and
/// the process is not root, so nothing happens.
pub fn chown_to_synthpanel(path: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let Ok(cpath) = CString::new(path.as_os_str().as_bytes()) else {
        return;
    };
    unsafe {
        let user = CString::new(WEB_USER).unwrap();
        let group = CString::new(WEB_USER).unwrap();
        let pw = libc::getpwnam(user.as_ptr());
        let gr = libc::getgrnam(group.as_ptr());
        if pw.is_null() || gr.is_null() {
            return;
        }
        libc::chown(cpath.as_ptr(), (*pw).pw_uid, (*gr).gr_gid);
    }
}
