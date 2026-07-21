//! Group ownership and file modes for everything the daemon exposes.
//!
//! The socket and the snapshot directory share one group boundary rather than
//! inventing two. Resolving a group name to a gid needs `getgrnam_r`, which has
//! no safe equivalent in std, so this is the only module in the crate — indeed
//! the only module in the workspace — that touches `unsafe`;
//! `#![deny(unsafe_code)]` on the crate root makes that a build failure rather
//! than a convention. Changing ownership and mode go through
//! `std::os::unix::fs::chown` and `std::fs::set_permissions`, both entirely
//! safe.

use anyhow::{anyhow, Context, Result};
use std::ffi::CString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Process-wide settings resolved from the command line rather than the config
/// file, so they cannot be changed over the socket they protect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Privileges {
    /// Group applied to the socket and to snapshots, when `--socket-group` was
    /// given. `None` means "leave the process's own group", which is already
    /// correct under the systemd unit.
    pub group_gid: Option<u32>,
}

/// Resolve a group name to its gid.
pub fn resolve_gid(name: &str) -> Result<u32> {
    let c_name =
        CString::new(name).with_context(|| format!("group name {name:?} contains a NUL byte"))?;

    // getgrnam_r fills a caller-provided buffer. 16 KiB is far past any real
    // /etc/group line, and we treat ERANGE as an error rather than growing:
    // a group that large is a misconfiguration, not a case to accommodate.
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: `libc::group` is POD — three raw pointers (`gr_name`, `gr_passwd`,
    // `gr_mem`) plus a `gid_t` (`gr_gid`) — with no niche or non-null
    // requirement on any field, so the all-zero bit pattern is a valid value.
    // `getgrnam_r` below fully populates it before we read from it.
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::group = std::ptr::null_mut();

    // SAFETY: `c_name` is a valid NUL-terminated string, `grp` and `found` are
    // valid out-pointers we own, and `buf` is a writable buffer of the length
    // we pass. getgrnam_r writes only within those bounds.
    let rc = unsafe {
        libc::getgrnam_r(
            c_name.as_ptr(),
            &mut grp,
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };

    if rc != 0 {
        // POSIX permits getgrnam_r to report "not found" as a nonzero rc with
        // one of these errnos, rather than rc == 0 with a NULL `found`. glibc's
        // `files` backend always does the latter, but sssd and ldap backends
        // commonly do the former. ERANGE must stay a genuine error below: it
        // means our buffer was too small, not that the group doesn't exist.
        if matches!(rc, libc::ENOENT | libc::ESRCH | libc::EBADF | libc::EPERM) {
            return Err(anyhow!("no such group: {name}"));
        }
        return Err(std::io::Error::from_raw_os_error(rc))
            .with_context(|| format!("looking up group {name}"));
    }
    if found.is_null() {
        return Err(anyhow!("no such group: {name}"));
    }
    Ok(grp.gr_gid)
}

/// Set the group owner of `path`, leaving the user owner untouched.
pub fn chgrp(path: &Path, gid: u32) -> Result<()> {
    // `None` for the uid becomes chown(2)'s `(uid_t)-1` "leave unchanged"
    // sentinel; std performs the same call this module used to make via
    // `libc::chown` directly, without needing `unsafe` here.
    std::os::unix::fs::chown(path, None, Some(gid))
        .with_context(|| format!("setting group {gid} on {}", path.display()))
}

/// Set the mode of `path`.
pub fn chmod(path: &Path, mode: u32) -> Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting mode {mode:o} on {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn resolves_a_group_that_always_exists() {
        // `root` is gid 0 on every Linux system.
        assert_eq!(resolve_gid("root").unwrap(), 0);
    }

    #[test]
    fn an_unknown_group_is_an_error_naming_it() {
        let err = resolve_gid("definitely-not-a-real-group-9f3a").unwrap_err();
        assert!(
            err.to_string().contains("definitely-not-a-real-group-9f3a"),
            "error should name the group, got: {err}"
        );
    }

    #[test]
    fn a_group_name_with_a_nul_byte_is_rejected() {
        assert!(resolve_gid("bad\0name").is_err());
    }

    #[test]
    fn chmod_sets_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();

        chmod(&file, 0o640).unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640, "got {mode:o}");
    }

    #[test]
    fn chgrp_to_our_own_group_succeeds_and_leaves_the_owner_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();

        let before = std::fs::metadata(&file).unwrap();
        // Changing to the group we already have needs no privilege.
        let own_gid = before.gid();
        chgrp(&file, own_gid).unwrap();

        let after = std::fs::metadata(&file).unwrap();
        assert_eq!(after.gid(), own_gid);
        assert_eq!(after.uid(), before.uid(), "chgrp must not touch the owner");
    }

    /// The same-group case above would still pass a `chgrp` that is a no-op, or
    /// one with its uid/gid arguments swapped (indistinguishable from a no-op
    /// whenever uid == gid, which is the common user-private-group default).
    /// Actually changing to a *different*, supplementary group we belong to is
    /// the discriminating case; `getgroups(2)` needs no privilege to read.
    /// Guarded so this never requires membership in more than one group, and
    /// never requires root.
    #[test]
    fn chgrp_to_a_different_supplementary_group_actually_changes_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();

        let before = std::fs::metadata(&file).unwrap();
        let own_gid = before.gid();

        let mut gs = [0 as libc::gid_t; 64];
        // SAFETY: writing at most gs.len() gid_t values into a buffer we own.
        let n = unsafe { libc::getgroups(gs.len() as libc::c_int, gs.as_mut_ptr()) };
        if let Some(&other) = gs[..n.max(0) as usize].iter().find(|&&g| g != own_gid) {
            chgrp(&file, other).unwrap();
            let after = std::fs::metadata(&file).unwrap();
            assert_eq!(after.gid(), other, "chgrp must change the group");
            assert_eq!(after.uid(), before.uid(), "chgrp must not touch the owner");
        }
        // Else: this user belongs to only one group on this machine, so there is
        // no second group to discriminate against without root. Nothing to
        // assert in that case.
    }

    #[test]
    fn chgrp_on_a_missing_path_names_the_path() {
        let err = chgrp(std::path::Path::new("/nonexistent/alertu/x"), 0).unwrap_err();
        assert!(
            err.to_string().contains("/nonexistent/alertu/x"),
            "got: {err}"
        );
    }

    #[test]
    fn chmod_on_a_missing_path_names_the_path() {
        let err = chmod(std::path::Path::new("/nonexistent/alertu/x"), 0o640).unwrap_err();
        assert!(
            err.to_string().contains("/nonexistent/alertu/x"),
            "got: {err}"
        );
    }
}
