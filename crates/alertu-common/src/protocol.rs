//! The line-delimited JSON protocol spoken over the Unix socket between the
//! daemon and the per-session GUI client.
//!
//! Framing: one JSON value per line (`\n`-terminated). This keeps the wire
//! format trivial to produce/consume with buffered line reads and easy to
//! inspect with `socat`/`nc` while debugging.

use crate::config::Config;
use crate::state::GuardState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default socket path. The daemon runs as root/system and places the socket in
/// a world-reachable runtime dir; permissions on the socket restrict who may
/// connect.
pub const DEFAULT_SOCKET_PATH: &str = "/run/alertu/alertu.sock";

/// Metadata about an input device, as reported by the daemon (which is the
/// component that actually has `input`-group access).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDeviceInfo {
    /// evdev node path, e.g. `/dev/input/event7`.
    pub path: PathBuf,
    /// Human-readable device name reported by the kernel.
    pub name: String,
    /// Looks like a keyboard (has `EV_KEY` with alnum keys).
    pub is_keyboard: bool,
    /// Looks like a relative pointer (has `REL_X`/`REL_Y` + `BTN_LEFT`) — the
    /// system's main mouse, excluded from the remote picker by default.
    pub is_pointer: bool,
}

/// Requests sent GUI → daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Ask for the current state (answered with [`Response::State`]).
    GetState,
    /// Register this connection to receive [`Response::StateChanged`] pushes.
    Subscribe,
    /// Force-arm regardless of remote (locks the session).
    Arm,
    /// Force-disarm (unlocks the session).
    Disarm,
    /// Toggle arm/disarm, same as a remote button press.
    Toggle,
    /// Fetch the daemon's current effective config.
    GetConfig,
    /// Replace the daemon's config, persist it, and re-apply live.
    SetConfig(Box<Config>),
    /// Enumerate available input devices (daemon-side `evdev::enumerate`).
    ListDevices,
    /// Watch every input device and report the first key press seen, so a remote
    /// can be paired by pressing its button.
    ///
    /// The daemon only *reports* what it saw; applying it is an ordinary
    /// [`Request::SetConfig`] from the client. Pairing therefore needs no new
    /// write path, and a client can show the user what was detected before
    /// committing to it.
    LearnRemote { timeout_secs: u64 },
    /// Play the beep once, through the daemon.
    ///
    /// The daemon is the only process that can prove the alarm is audible: it
    /// runs as its own user, with its own groups and its own `alsa_device`, so a
    /// sound the *client* can play says nothing about whether the siren will be
    /// heard. Without this, checking the audio output means arming for real and
    /// locking the screen to hear a chirp.
    TestSound,
}

/// Replies and asynchronous pushes sent daemon → GUI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Response {
    /// Snapshot of the current state (reply to `GetState`/`Arm`/… ).
    State { state: GuardState },
    /// Asynchronous push after a state transition (to subscribers).
    StateChanged { state: GuardState },
    /// Current config (reply to `GetConfig`).
    Config(Box<Config>),
    /// Device list (reply to `ListDevices`).
    Devices { devices: Vec<InputDeviceInfo> },
    /// A key press was observed (reply to `LearnRemote`): which device sent it
    /// and which evdev key name it was, ready to be written into a `Config`.
    Learned {
        path: PathBuf,
        name: String,
        key: String,
    },
    /// The learning window closed with no key press (reply to `LearnRemote`).
    LearnTimedOut,
    /// Generic acknowledgement (reply to `SetConfig`, etc.).
    Ok,
    /// An error string describing why the last request failed.
    Error { message: String },
}
