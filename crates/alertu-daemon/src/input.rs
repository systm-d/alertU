//! Per-device evdev reader tasks.
//!
//! Each watched/remote device gets its own async task that translates raw
//! `InputEvent`s into high-level [`InputSignal`]s on a shared channel. Tasks are
//! aborted and re-spawned when the configuration changes.

use evdev::{Device, EventSummary, KeyCode};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// A high-level signal derived from raw input events.
#[derive(Debug, Clone)]
pub enum InputSignal {
    /// The remote's toggle key was pressed.
    RemoteToggle,
    /// Activity happened on a watched device (potential intrusion).
    Activity { source: String },
}

/// Role a reader plays.
enum Role {
    /// The remote: only presses of one of these keys matter.
    Remote(Vec<KeyCode>),
    /// A watched device: any key press or pointer motion is activity.
    Watch,
}

/// Spawn a reader task for the remote device.
pub fn spawn_remote(
    path: PathBuf,
    toggle_keys: Vec<KeyCode>,
    tx: mpsc::Sender<InputSignal>,
) -> JoinHandle<()> {
    tokio::spawn(reader(path, Role::Remote(toggle_keys), tx))
}

/// Spawn a reader task for a watched device.
pub fn spawn_watch(path: PathBuf, tx: mpsc::Sender<InputSignal>) -> JoinHandle<()> {
    tokio::spawn(reader(path, Role::Watch, tx))
}

async fn reader(path: PathBuf, role: Role, tx: mpsc::Sender<InputSignal>) {
    let dev_path = path.display().to_string();

    let device = match Device::open(&path) {
        Ok(d) => d,
        Err(e) => {
            warn!(device = %dev_path, error = %e, "cannot open input device");
            return;
        }
    };
    let name = device.name().unwrap_or("<unnamed>").to_string();

    let mut stream = match device.into_event_stream() {
        Ok(s) => s,
        Err(e) => {
            warn!(device = %dev_path, error = %e, "cannot stream input device");
            return;
        }
    };

    match &role {
        Role::Remote(keys) => info!(device = %dev_path, %name, keys = ?keys, "watching remote"),
        Role::Watch => info!(device = %dev_path, %name, "watching for intrusion"),
    }

    loop {
        let event = match stream.next_event().await {
            Ok(ev) => ev,
            Err(e) => {
                // Device unplugged or read error — stop this reader; the
                // supervisor will re-spawn on the next reconfigure.
                warn!(device = %dev_path, error = %e, "input stream ended");
                return;
            }
        };

        let signal = match &role {
            Role::Remote(keys) => match event.destructure() {
                // value == 1 is a key press (2 = autorepeat, 0 = release).
                EventSummary::Key(_, code, 1) if keys.contains(&code) => {
                    Some(InputSignal::RemoteToggle)
                }
                _ => None,
            },
            Role::Watch => match event.destructure() {
                EventSummary::Key(_, _, 1) => Some(InputSignal::Activity {
                    source: name.clone(),
                }),
                EventSummary::RelativeAxis(_, _, v) if v != 0 => Some(InputSignal::Activity {
                    source: name.clone(),
                }),
                EventSummary::AbsoluteAxis(_, _, _) => Some(InputSignal::Activity {
                    source: name.clone(),
                }),
                _ => None,
            },
        };

        if let Some(sig) = signal {
            debug!(device = %dev_path, ?sig, "input signal");
            if tx.send(sig).await.is_err() {
                // Receiver gone — the machine shut down.
                return;
            }
        }
    }
}
