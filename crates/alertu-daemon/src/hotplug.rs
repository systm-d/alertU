//! Device hotplug detection.
//!
//! Watches `/dev/input` with inotify and asks the state machine to rescan when
//! device nodes appear or disappear, so plugging/unplugging a remote (or any
//! watched device) takes effect without a manual refresh. Debounced, because a
//! single plug event typically produces a `CREATE` followed by an `ATTRIB` once
//! udev applies group permissions.
//!
//! inotify is used directly (pure-Rust, no libudev) to keep the daemon free of
//! a C library dependency, consistent with the rest of the project.

use crate::machine::Control;
use futures_util::StreamExt;
use inotify::{Inotify, WatchMask};
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Directory holding the evdev nodes.
pub const INPUT_DIR: &str = "/dev/input";

/// Quiet window after the last change before a rescan is issued.
const DEBOUNCE: Duration = Duration::from_millis(600);

/// Watch `dir` forever, sending [`Control::Rescan`] after bursts of device-node
/// changes settle. Any setup failure disables hotplug (logged) but never brings
/// the daemon down — manual "Refresh" from the GUI still works.
pub async fn watch<P: AsRef<Path>>(dir: P, ctrl_tx: mpsc::Sender<Control>) {
    let dir = dir.as_ref();

    let inotify = match Inotify::init() {
        Ok(i) => i,
        Err(e) => {
            warn!(error = %e, "inotify init failed; hotplug disabled");
            return;
        }
    };

    let mask = WatchMask::CREATE
        | WatchMask::DELETE
        | WatchMask::MOVED_FROM
        | WatchMask::MOVED_TO
        | WatchMask::ATTRIB;
    if let Err(e) = inotify.watches().add(dir, mask) {
        warn!(error = %e, dir = %dir.display(), "cannot watch input dir; hotplug disabled");
        return;
    }

    let mut stream = match inotify.into_event_stream([0u8; 1024]) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "inotify stream failed; hotplug disabled");
            return;
        }
    };
    info!(dir = %dir.display(), "hotplug watcher active");

    let mut dirty = false;
    loop {
        let debounce = tokio::time::sleep(DEBOUNCE);
        tokio::pin!(debounce);

        tokio::select! {
            event = stream.next() => match event {
                Some(Ok(ev)) => {
                    debug!(name = ?ev.name, "input dir changed");
                    dirty = true;
                }
                Some(Err(e)) => warn!(error = %e, "inotify read error"),
                None => {
                    warn!("inotify stream ended; hotplug stopped");
                    return;
                }
            },
            _ = &mut debounce, if dirty => {
                dirty = false;
                debug!("rescanning input devices after hotplug");
                if ctrl_tx.send(Control::Rescan).await.is_err() {
                    return; // machine gone
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn node_creation_triggers_a_rescan() {
        let dir = std::env::temp_dir().join(format!("alertu-hotplug-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let (tx, mut rx) = mpsc::channel(4);
        let watch_dir = dir.clone();
        let handle = tokio::spawn(async move { watch(watch_dir, tx).await });

        // Let the watcher arm before we poke the directory.
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::fs::write(dir.join("event99"), b"x").unwrap();

        let got = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await;

        handle.abort();
        std::fs::remove_dir_all(&dir).ok();

        assert!(
            matches!(got, Ok(Some(Control::Rescan))),
            "expected a Rescan control message after a node appeared in the watched dir"
        );
    }
}
