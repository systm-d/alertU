//! Input-device enumeration and classification.
//!
//! The daemon is the component with `input`-group access, so it owns device
//! discovery and reports the list to the GUI over IPC.

use alertu_common::config::Config;
use alertu_common::protocol::InputDeviceInfo;
use evdev::{Device, KeyCode, RelativeAxisCode};
use std::path::PathBuf;

/// A discovered input device with the traits the UI cares about.
#[derive(Debug, Clone)]
pub struct DeviceEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_keyboard: bool,
    pub is_pointer: bool,
}

impl DeviceEntry {
    pub fn to_info(&self) -> InputDeviceInfo {
        InputDeviceInfo {
            path: self.path.clone(),
            name: self.name.clone(),
            is_keyboard: self.is_keyboard,
            is_pointer: self.is_pointer,
        }
    }
}

/// Classify a device by its declared capabilities.
///
/// * pointer  — advertises `REL_X`/`REL_Y` and `BTN_LEFT` (the main mouse).
/// * keyboard — advertises at least one ordinary typing/navigation key.
///
/// A device can be both, neither, or one of the two. These are heuristics used
/// only for the UI (graying out the main mouse, labelling entries); the actual
/// remote/watch roles are always driven by explicit config.
fn classify(dev: &Device) -> (bool, bool) {
    let keys = dev.supported_keys();
    let rels = dev.supported_relative_axes();

    let is_pointer = rels.is_some_and(|r| {
        r.contains(RelativeAxisCode::REL_X) && r.contains(RelativeAxisCode::REL_Y)
    }) && keys.is_some_and(|k| k.contains(KeyCode::BTN_LEFT));

    let is_keyboard = keys.is_some_and(|k| {
        k.contains(KeyCode::KEY_ENTER)
            || k.contains(KeyCode::KEY_A)
            || k.contains(KeyCode::KEY_SPACE)
            || k.contains(KeyCode::KEY_VOLUMEUP)
    });

    (is_keyboard, is_pointer)
}

/// Enumerate all readable `/dev/input/event*` devices, sorted by path.
///
/// Devices that fail to open (permissions, races) are skipped rather than
/// aborting the whole listing.
pub fn list() -> Vec<DeviceEntry> {
    let mut entries: Vec<DeviceEntry> = evdev::enumerate()
        .map(|(path, dev)| {
            let (is_keyboard, is_pointer) = classify(&dev);
            DeviceEntry {
                path,
                name: dev.name().unwrap_or("<unnamed>").to_string(),
                is_keyboard,
                is_pointer,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

/// The concrete devices to open, resolved from config against a device snapshot.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    /// Path of the remote, if one could be resolved.
    pub remote: Option<PathBuf>,
    /// Friendly name of the resolved remote (for logging).
    pub remote_name: Option<String>,
    /// Devices to watch for intrusion (never includes the remote).
    pub watch: Vec<PathBuf>,
}

/// Resolve the remote and watch-device paths from `cfg`.
///
/// * remote: an explicit path, or the first device whose name contains
///   `remote_name_hint` (case-insensitive) when `remote_device = "auto"`.
/// * watch: an explicit list, or — when `watch_devices = ["auto"]` — every
///   device except the remote and the main pointer. The remote is always
///   excluded from the watch set.
pub fn resolve(cfg: &Config, entries: &[DeviceEntry]) -> Resolved {
    let (remote, remote_name) = if cfg.remote_is_auto() {
        let hint = cfg.remote_name_hint.to_lowercase();
        match entries
            .iter()
            .find(|e| e.name.to_lowercase().contains(&hint))
        {
            Some(e) => (Some(e.path.clone()), Some(e.name.clone())),
            None => (None, None),
        }
    } else {
        let path = PathBuf::from(&cfg.remote_device);
        let name = entries
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.name.clone());
        (Some(path), name)
    };

    let watch = if cfg.watch_is_auto() {
        entries
            .iter()
            .filter(|e| Some(&e.path) != remote.as_ref() && !e.is_pointer)
            .map(|e| e.path.clone())
            .collect()
    } else {
        cfg.watch_devices
            .iter()
            .map(PathBuf::from)
            .filter(|p| Some(p) != remote.as_ref())
            .collect()
    };

    Resolved {
        remote,
        remote_name,
        watch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, name: &str, is_keyboard: bool, is_pointer: bool) -> DeviceEntry {
        DeviceEntry {
            path: PathBuf::from(path),
            name: name.to_string(),
            is_keyboard,
            is_pointer,
        }
    }

    fn sample() -> Vec<DeviceEntry> {
        vec![
            entry("/dev/input/event0", "AB Shutter 3", true, false),
            entry("/dev/input/event1", "Logitech USB Mouse", false, true),
            entry(
                "/dev/input/event2",
                "AT Translated Set 2 keyboard",
                true,
                false,
            ),
        ]
    }

    #[test]
    fn auto_resolves_remote_by_hint_and_watches_the_rest() {
        let cfg = Config::default(); // hint "AB Shutter", watch auto
        let r = resolve(&cfg, &sample());
        assert_eq!(r.remote, Some(PathBuf::from("/dev/input/event0")));
        // Pointer (mouse) and the remote itself are excluded.
        assert_eq!(r.watch, vec![PathBuf::from("/dev/input/event2")]);
    }

    #[test]
    fn explicit_remote_is_never_in_the_watch_list() {
        let mut cfg = Config::default();
        cfg.remote_device = "/dev/input/event0".into();
        cfg.watch_devices = vec![
            "/dev/input/event0".into(), // remote, must be filtered out
            "/dev/input/event2".into(),
        ];
        let r = resolve(&cfg, &sample());
        assert_eq!(r.remote, Some(PathBuf::from("/dev/input/event0")));
        assert_eq!(r.watch, vec![PathBuf::from("/dev/input/event2")]);
    }

    #[test]
    fn unknown_hint_yields_no_remote_but_still_watches() {
        let mut cfg = Config::default();
        cfg.remote_name_hint = "Nonexistent Remote".into();
        let r = resolve(&cfg, &sample());
        assert_eq!(r.remote, None);
        // With no remote resolved, every non-pointer device is watched.
        assert_eq!(
            r.watch,
            vec![
                PathBuf::from("/dev/input/event0"),
                PathBuf::from("/dev/input/event2"),
            ]
        );
    }
}
