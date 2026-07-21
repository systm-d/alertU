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
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use tracing::warn;

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

/// The effective uid of this process.
///
/// Callers need it to decide whether a directory is theirs to manage before
/// changing its mode or group: chmod-ing a directory the daemon does not own —
/// `/tmp`, or a working directory a developer pointed `--socket` into — is
/// vandalism, not hardening. `geteuid` has no safe wrapper in std, so like the
/// group lookup above it lives in this module.
pub fn effective_uid() -> u32 {
    // SAFETY: `geteuid` takes no arguments and reads no caller memory, so there
    // are no pointers to get wrong. POSIX gives it no error return: it cannot
    // fail and always yields a valid uid.
    unsafe { libc::geteuid() }
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

/// The mode a managed directory carries *while* the daemon is still putting
/// things inside it: owner-only, whatever the process umask would have given.
const DIR_SETUP_MODE: u32 = 0o700;

/// The mode a managed directory ends up at: traversable by the boundary's
/// group, never by everyone else.
const DIR_MODE: u32 = 0o750;

/// Create `dir` if needed and apply the group boundary — but only if the
/// directory is ours to manage. Returns whether the boundary was applied.
///
/// Ordering matters: owner-only while we set up, then the group, then widened.
/// A directory we do not own is left alone with a warning, because chmod sets an
/// absolute mode and would as easily wreck `/tmp` as tighten our own directory.
pub fn secure_dir(dir: &Path, privileges: Privileges) -> Result<bool> {
    if !begin_secure_dir(dir, privileges)? {
        return Ok(false);
    }
    finish_secure_dir(dir)?;
    Ok(true)
}

/// The first half of [`secure_dir`]: create the directory, decide whether it is
/// ours, and — if it is — narrow it to owner-only and apply the group. Returns
/// whether the directory is ours to manage, i.e. whether the caller must still
/// call [`finish_secure_dir`].
///
/// Split from the widening step for the control socket, which must stay at
/// `0700` across its `bind` so the socket is never briefly reachable by the
/// wrong group; everything else wants both halves at once via [`secure_dir`].
///
/// The narrow-then-group-then-widen order is not cosmetic. `create_dir_all` and
/// the first `chmod` are separate syscalls, and in between the directory carries
/// whatever the umask produced; and until the `chgrp` lands it still carries the
/// daemon's own primary group, which under a hand-rolled install can be a shared
/// group like `users`. Widening straight to `0750` would open the directory to
/// that wrong group for the duration of setup. The group goes on first, the
/// traversal bit last.
pub fn begin_secure_dir(dir: &Path, privileges: Privileges) -> Result<bool> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating directory {}", dir.display()))?;

    // `symlink_metadata`, so a symlink planted where the directory should be is
    // judged on its own ownership rather than its target's.
    let meta = std::fs::symlink_metadata(dir)
        .with_context(|| format!("inspecting directory {}", dir.display()))?;
    if !meta.is_dir() || meta.uid() != effective_uid() {
        // Not a failure: an operator-supplied path very often lands in a
        // directory belonging to somebody else — `/tmp` (`1777`, and chmod-ing
        // it would destroy the sticky bit machine-wide), a developer's working
        // directory, `$HOME`, or a `/var/lib/alertu/snapshots` an admin
        // pre-created as root. Refusing there would make the daemon unrunnable
        // by hand and would lose alarm evidence outright; the modes we set on
        // the socket and on each captured file apply regardless of who owns the
        // directory around them.
        warn!(
            dir = %dir.display(),
            "directory is not owned by this daemon; its permissions cannot be secured. \
             The modes alertu sets on the socket and on snapshot files still apply."
        );
        return Ok(false);
    }

    chmod(dir, DIR_SETUP_MODE)?;
    if let Some(gid) = privileges.group_gid {
        chgrp(dir, gid)?;
    }
    Ok(true)
}

/// The second half of [`secure_dir`]: widen a directory [`begin_secure_dir`]
/// claimed to its final, group-traversable mode.
pub fn finish_secure_dir(dir: &Path) -> Result<()> {
    chmod(dir, DIR_MODE)
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

    /// The uid we report must be the one the filesystem attributes our files
    /// to, since that is the comparison `ipc::bind` makes to decide whether a
    /// directory is ours to chmod. A constant, or a `getuid`/`geteuid` mix-up
    /// under a setuid binary, would fail here.
    #[test]
    fn effective_uid_is_the_uid_our_own_files_get() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().uid(), effective_uid());
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

    /// The whole point of `secure_dir`: a directory we own ends up group-
    /// traversable and not world-anything, created if it was missing.
    #[test]
    fn secure_dir_applies_the_boundary_to_a_directory_we_own() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("snapshots");

        assert!(
            secure_dir(&target, Privileges::default()).unwrap(),
            "a directory we just created is ours to manage"
        );

        let meta = std::fs::metadata(&target).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o750, "dir mode was {mode:o}, expected 0750");
    }

    /// With an explicit group the directory must carry it, or a group member
    /// cannot traverse in and the boundary is silently inoperative. Uses a
    /// supplementary group we already belong to, so this needs no privilege;
    /// skipped when this account has only one group.
    #[test]
    fn secure_dir_applies_the_requested_group() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("snapshots");
        let own_gid = std::fs::metadata(dir.path()).unwrap().gid();

        let mut gs = [0 as libc::gid_t; 64];
        // SAFETY: writing at most gs.len() gid_t values into a buffer we own.
        let n = unsafe { libc::getgroups(gs.len() as libc::c_int, gs.as_mut_ptr()) };
        let Some(&other) = gs[..n.max(0) as usize].iter().find(|&&g| g != own_gid) else {
            return; // only one group here; nothing that would discriminate
        };

        assert!(secure_dir(
            &target,
            Privileges {
                group_gid: Some(other)
            }
        )
        .unwrap());

        let meta = std::fs::metadata(&target).unwrap();
        assert_eq!(meta.gid(), other, "the group boundary was not applied");
        assert_eq!(meta.permissions().mode() & 0o777, 0o750);
    }

    /// The guard that keeps this from being a vandalism primitive: `/` is
    /// root's, so `secure_dir` must decline it rather than chmod it. Under an
    /// euid of 0 every directory is legitimately "ours", so there is nothing to
    /// assert — and making a directory owned by somebody else needs privilege
    /// these tests deliberately never take. Skipped rather than faked.
    #[test]
    fn secure_dir_declines_a_directory_we_do_not_own() {
        if effective_uid() == 0 {
            return;
        }
        let root = Path::new("/");
        let before = std::fs::metadata(root).unwrap().permissions().mode();

        assert!(
            !secure_dir(root, Privileges::default()).unwrap(),
            "a directory owned by root must be declined, not chmod-ed"
        );

        let after = std::fs::metadata(root).unwrap().permissions().mode();
        assert_eq!(before, after, "the foreign directory's mode was changed");
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
