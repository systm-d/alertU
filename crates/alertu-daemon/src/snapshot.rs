//! Webcam capture via an external tool (`fswebcam` or `ffmpeg`).

use crate::perms::{self, Privileges};
use alertu_common::config::Config;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{info, warn};

/// Capture a single still to `snapshot_dir`, named with a timestamp, and return
/// its path. Prefers `fswebcam`, falling back to `ffmpeg`.
pub async fn capture(cfg: &Config, privileges: Privileges) -> Result<PathBuf> {
    tokio::fs::create_dir_all(&cfg.snapshot_dir)
        .await
        .with_context(|| format!("creating snapshot dir {}", cfg.snapshot_dir.display()))?;

    // Alarm photographs are of whoever is at the machine — including the owner.
    // They get the same group boundary as the control socket, and deliberately
    // not 0644: world-readable webcam stills would be a privacy regression.
    //
    // Owner-only until the group is right, same order as `ipc::finish_socket`:
    // `create_dir_all` and the `chmod` below are separate syscalls, and between
    // them the directory carries whatever group `create_dir_all` gave it — the
    // daemon's own primary group, which under a hand-rolled install can be a
    // shared group like `users`. Widening straight to `0750` would open alarm
    // photographs to that wrong group for the duration of setup. The group
    // goes on first, the traversal bit last.
    perms::chmod(&cfg.snapshot_dir, 0o700)?;
    if let Some(gid) = privileges.group_gid {
        perms::chgrp(&cfg.snapshot_dir, gid)?;
    }
    perms::chmod(&cfg.snapshot_dir, 0o750)?;

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let out = cfg.snapshot_dir.join(format!("alertu-{stamp}.jpg"));

    let result = if which("fswebcam") {
        Command::new("fswebcam")
            .args(["-d", &cfg.camera_device, "-r", "1280x720", "--no-banner"])
            .arg(&out)
            .output()
            .await
    } else if which("ffmpeg") {
        Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "v4l2",
                "-i",
                &cfg.camera_device,
                "-frames:v",
                "1",
            ])
            .arg(&out)
            .output()
            .await
    } else {
        return Err(anyhow!(
            "no capture tool found (install fswebcam or ffmpeg)"
        ));
    };

    let output = result.context("running capture tool")?;
    if output.status.success() {
        secure_captured_file(&out, privileges)?;
        info!(file = %out.display(), "snapshot captured");
        Ok(out)
    } else {
        Err(anyhow!(
            "capture failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Apply the snapshot boundary to a freshly captured file.
///
/// Split out from `capture` so it can be tested: the capture itself needs a real
/// camera, this does not.
///
/// Unlike the directory above, chmod-then-chgrp here has no ordering hazard: the
/// file does not exist until the capture tool just finished writing it, so there
/// is no window in which some other, wrong-group reader could see it —
/// narrowing the mode first only ever shrinks exposure, never widens it.
fn secure_captured_file(out: &Path, privileges: Privileges) -> Result<()> {
    // The capture tool created the file under its own umask, so fix the mode
    // afterwards rather than trying to control the child's umask.
    perms::chmod(out, 0o640)?;
    if let Some(gid) = privileges.group_gid {
        perms::chgrp(out, gid)?;
    }
    Ok(())
}

/// Spawn a capture in the background so the state machine isn't blocked.
pub fn capture_async(cfg: Config, privileges: Privileges) {
    tokio::spawn(async move {
        if let Err(e) = capture(&cfg, privileges).await {
            warn!(error = %e, "snapshot capture failed");
        }
    });
}

fn which(prog: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(prog).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// `secure_captured_file`'s `chmod` sets an absolute mode, so in principle
    /// it does not depend on the process umask at all — but a naive version of
    /// this test that forgot the `chmod` call (or dropped it by accident) could
    /// still pass by coincidence: this sandbox's ambient umask (`0027`) makes a
    /// bare `std::fs::write` come out `0640` too, the same value this test
    /// checks for.
    ///
    /// `umask(2)` is process-global and the test harness is multi-threaded, so
    /// setting it in-process would race every other test. Instead this test
    /// re-runs itself in a child process under a fixed `umask 022`, under which
    /// a bare `fs::write` would come out `0644`, not `0640` — a value that
    /// actually discriminates "the chmod ran" from "the chmod didn't run",
    /// whatever the ambient umask of the machine invoking `cargo test` happens
    /// to be. Same technique as `alertu-ctl`'s
    /// `gen_sounds_writes_readable_files_whatever_the_umask`.
    #[test]
    fn secure_captured_file_sets_the_mode_whatever_the_umask() {
        const MARKER: &str = "ALERTU_SNAPSHOT_FILE_UMASK_CHILD";

        if std::env::var_os(MARKER).is_none() {
            let exe = std::env::current_exe().unwrap();
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "umask 022; exec {exe:?} --exact --nocapture \
                     snapshot::tests::secure_captured_file_sets_the_mode_whatever_the_umask"
                ))
                .env(MARKER, "1")
                .status()
                .expect("re-running the test binary under a fixed umask");
            assert!(status.success(), "the run under umask 022 failed");
            return;
        }

        // Child: a plain temp file, no camera or capture tool involved.
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("alertu-snapshot.jpg");
        std::fs::write(&out, b"not a real jpeg").unwrap();

        secure_captured_file(&out, Privileges::default()).unwrap();

        let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o640,
            "captured file mode was {mode:o}, expected 0640"
        );
    }
}
