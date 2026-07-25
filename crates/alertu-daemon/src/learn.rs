//! "Press a button on your remote" pairing.
//!
//! Pairing used to mean reading `alertu-ctl list-devices`, guessing which
//! `/dev/input/eventN` is the remote, then guessing which evdev key name its
//! button sends — and a wrong key produces no error anywhere, just a remote that
//! never arms. This module removes both guesses: it watches every device at once
//! and reports the first key press it sees.
//!
//! It lives in the daemon because the daemon is the only component with
//! `input`-group access, the same reason device enumeration lives here.
//!
//! Reporting is all it does. The observed device and key travel back to the
//! client, which writes them with an ordinary `SetConfig`, so pairing introduces
//! no second way to modify the configuration.

use crate::devices::DeviceEntry;
use evdev::{Device, EventSummary, KeyCode};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout_at};
use tracing::{debug, info, warn};

/// Presses arriving within this window of the request are discarded.
///
/// The user reached the pairing button with a mouse click or the Enter key, and
/// the corresponding press event is still in flight. Without this it would be
/// learned as the remote's button — reliably, since it arrives first.
const SETTLE: Duration = Duration::from_millis(600);

/// What the pairing attempt saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A key press, ready to be written into a config.
    Learned {
        path: PathBuf,
        name: String,
        key: String,
    },
    /// The window closed with nothing pressed.
    TimedOut,
}

/// One observed key press.
struct Press {
    path: PathBuf,
    name: String,
    key: String,
}

/// Watch every non-pointer device and answer with the first key press.
///
/// Runs detached from the state machine: the machine must stay responsive to
/// `arm`/`disarm`/`status` while a client sits in a pairing dialog.
pub async fn run(
    entries: Vec<DeviceEntry>,
    window: Duration,
    reply: oneshot::Sender<Result<Outcome, String>>,
) {
    let (tx, mut rx) = mpsc::channel::<Press>(16);

    let mut readers = Vec::new();
    for entry in entries {
        // The main mouse is excluded, not filtered later: its motion is
        // harmless, but `BTN_LEFT` from the very click that opened the dialog
        // would be indistinguishable from a deliberate press.
        if entry.is_pointer {
            continue;
        }
        readers.push(tokio::spawn(watch_one(entry, tx.clone())));
    }
    drop(tx);

    if readers.is_empty() {
        let _ = reply.send(Err("no input device available to learn from".to_string()));
        return;
    }

    info!(
        devices = readers.len(),
        window_secs = window.as_secs(),
        "learning a remote: waiting for a key press"
    );

    let started = Instant::now();
    let deadline = started + window;
    let outcome = loop {
        match timeout_at(deadline, rx.recv()).await {
            Ok(Some(press)) if started.elapsed() < SETTLE => {
                debug!(key = %press.key, "ignoring a press inside the settle window");
            }
            Ok(Some(press)) => {
                info!(
                    device = %press.path.display(),
                    name = %press.name,
                    key = %press.key,
                    "learned a remote"
                );
                break Outcome::Learned {
                    path: press.path,
                    name: press.name,
                    key: press.key,
                };
            }
            // Every reader ended before the deadline: nothing left to hear from.
            Ok(None) => break Outcome::TimedOut,
            Err(_) => break Outcome::TimedOut,
        }
    };

    for reader in readers {
        reader.abort();
    }
    let _ = reply.send(Ok(outcome));
}

/// Forward every key press on one device to `tx` until the stream ends.
async fn watch_one(entry: DeviceEntry, tx: mpsc::Sender<Press>) {
    let device = match Device::open(&entry.path) {
        Ok(device) => device,
        Err(e) => {
            // Expected for nodes that vanished between enumeration and here.
            debug!(device = %entry.path.display(), error = %e, "cannot open device for learning");
            return;
        }
    };
    let mut stream = match device.into_event_stream() {
        Ok(stream) => stream,
        Err(e) => {
            warn!(device = %entry.path.display(), error = %e, "cannot stream device for learning");
            return;
        }
    };

    loop {
        let event = match stream.next_event().await {
            Ok(event) => event,
            Err(_) => return,
        };
        // value == 1 is a press; 2 is autorepeat and 0 is release.
        if let EventSummary::Key(_, code, 1) = event.destructure() {
            if is_pointer_button(code) {
                continue;
            }
            let press = Press {
                path: entry.path.clone(),
                name: entry.name.clone(),
                key: format!("{code:?}"),
            };
            if tx.send(press).await.is_err() {
                return; // pairing finished
            }
        }
    }
}

/// Whether `code` is a `BTN_*` button rather than a key.
///
/// Mouse and gamepad buttons occupy `0x100..0x160` (`BTN_MISC` through the last
/// `BTN_*`, just below `KEY_OK`). Learning one would bind the alarm to a stray
/// click on a device that is not a pointer overall — a trackball's extra
/// buttons, a tablet's stylus.
fn is_pointer_button(code: KeyCode) -> bool {
    (0x100..0x160).contains(&code.code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_buttons_are_rejected_and_keys_accepted() {
        assert!(is_pointer_button(KeyCode::BTN_LEFT));
        assert!(is_pointer_button(KeyCode::BTN_RIGHT));
        assert!(!is_pointer_button(KeyCode::KEY_VOLUMEUP));
        assert!(!is_pointer_button(KeyCode::KEY_ENTER));
        // KEY_OK (0x160) sits immediately above the BTN_* block; a remote that
        // reports it must remain pairable.
        assert!(!is_pointer_button(KeyCode::KEY_OK));
    }

    #[tokio::test]
    async fn no_usable_device_is_an_error_not_a_timeout() {
        let (tx, rx) = oneshot::channel();
        // A pointer-only list: every entry is skipped, so there is nothing to
        // listen on and the caller must be told rather than left waiting.
        let entries = vec![DeviceEntry {
            path: PathBuf::from("/dev/input/event0"),
            name: "Some Mouse".to_string(),
            is_keyboard: false,
            is_pointer: true,
        }];
        run(entries, Duration::from_millis(10), tx).await;
        assert!(matches!(rx.await, Ok(Err(_))));
    }

    #[tokio::test]
    async fn an_unopenable_device_times_out() {
        let (tx, rx) = oneshot::channel();
        let entries = vec![DeviceEntry {
            path: PathBuf::from("/nonexistent/event0"),
            name: "Gone".to_string(),
            is_keyboard: true,
            is_pointer: false,
        }];
        run(entries, Duration::from_millis(50), tx).await;
        assert_eq!(rx.await.unwrap().unwrap(), Outcome::TimedOut);
    }
}
