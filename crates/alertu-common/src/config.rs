//! Daemon configuration, loaded from a TOML file at startup and hot-reloadable
//! from the GUI over the IPC socket.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Sentinel value meaning "let the daemon pick automatically".
pub const AUTO: &str = "auto";

/// The full daemon configuration.
///
/// Every field has a sensible default so a missing/partial TOML still yields a
/// working configuration. Unknown fields are rejected to catch typos early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// evdev path of the device used as the remote (the "key fob"), e.g.
    /// `/dev/input/event7`, or `"auto"` to match by [`Config::remote_name_hint`].
    pub remote_device: String,

    /// Substring matched (case-insensitive) against device names when
    /// `remote_device = "auto"`.
    ///
    /// Empty by default: no model is assumed, and an empty hint deliberately
    /// resolves *nothing* rather than the first device that enumerates. Find
    /// yours with `alertu-ctl list-devices` and use any distinctive part of its
    /// name.
    pub remote_name_hint: String,

    /// evdev key name that arms/disarms, e.g. `KEY_VOLUMEUP` or `KEY_ENTER`.
    /// Any of the listed keys coming from the remote acts as the toggle.
    ///
    /// A name the remote cannot emit is not an error anywhere — it simply never
    /// matches, and the remote stays inert — so the daemon warns about that at
    /// startup (see `input::warn_about_unusable_keys`). Cheap Bluetooth
    /// shutters, the typical hardware here, mostly report `KEY_VOLUMEUP` and no
    /// `KEY_ENTER` at all, hence the default.
    pub toggle_keys: Vec<String>,

    /// Devices watched for intrusion while armed. Either an explicit list of
    /// evdev paths, or a single-element list `["auto"]` meaning "every input
    /// device except the remote and the system pointer".
    pub watch_devices: Vec<String>,

    /// Seconds to wait, after arming, before intrusion detection becomes live.
    /// Lets the user walk away without self-triggering.
    pub grace_period_secs: u64,

    /// Seconds between `Triggered` and the siren firing. A quiet warning tick
    /// plays during this window; disarming cancels everything.
    pub alarm_delay_secs: u64,

    /// Audio file played once on arm/disarm (car-style chirp).
    pub beep_sound: PathBuf,

    /// Audio file played, discreetly, while in the `Triggered` countdown.
    pub warning_sound: PathBuf,

    /// Audio file looped while in the `Alarm` state.
    pub siren_sound: PathBuf,

    /// ALSA device the sounds play on, e.g. `plughw:0,2`. Empty (the default)
    /// lets whichever player is found pick its own destination.
    ///
    /// Worth setting on a packaged install. The daemon runs as its own system
    /// user, and `paplay`/`pw-play` can only reach a PipeWire/PulseAudio server
    /// that lives *inside a user session* — which this user does not have, so
    /// they fail with "Host is down" and the alarm is mute. Naming an ALSA
    /// device routes playback straight at the hardware instead, which is what
    /// makes a system service audible at all. `aplay -L` lists the candidates;
    /// prefer the built-in speaker over a headset, since a siren playing into
    /// headphones on the desk protects nothing.
    pub alsa_device: String,

    /// Directory where webcam snapshots are written on `Alarm`.
    pub snapshot_dir: PathBuf,

    /// Camera device passed to the capture tool, e.g. `/dev/video0`.
    pub camera_device: String,

    /// Optional logind session id to control. `"auto"` (default) picks the
    /// active graphical seat session.
    pub session_id: String,

    /// Optional HTTP endpoint POSTed (via `curl`) when entering `Alarm`.
    /// Empty string disables the hook.
    pub alarm_webhook_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            remote_device: AUTO.to_string(),
            remote_name_hint: String::new(),
            toggle_keys: vec!["KEY_VOLUMEUP".to_string()],
            watch_devices: vec![AUTO.to_string()],
            grace_period_secs: 15,
            alarm_delay_secs: 10,
            beep_sound: PathBuf::from("/usr/share/sounds/alertu/beep.wav"),
            warning_sound: PathBuf::from("/usr/share/sounds/alertu/warning.wav"),
            siren_sound: PathBuf::from("/usr/share/sounds/alertu/siren.wav"),
            alsa_device: String::new(),
            snapshot_dir: PathBuf::from("/var/lib/alertu/snapshots"),
            camera_device: "/dev/video0".to_string(),
            session_id: AUTO.to_string(),
            alarm_webhook_url: String::new(),
        }
    }
}

impl Config {
    /// Whether the remote should be auto-detected by name.
    pub fn remote_is_auto(&self) -> bool {
        self.remote_device.eq_ignore_ascii_case(AUTO)
    }

    /// Whether the watch list is the "everything except remote/pointer" wildcard.
    pub fn watch_is_auto(&self) -> bool {
        matches!(self.watch_devices.as_slice(), [only] if only.eq_ignore_ascii_case(AUTO))
    }

    /// Whether a logind session id was pinned explicitly.
    pub fn session_is_auto(&self) -> bool {
        self.session_id.eq_ignore_ascii_case(AUTO)
    }

    /// The default on-disk config path (`$XDG_CONFIG_HOME/alertu/config.toml`,
    /// or `/etc/alertu/config.toml` when running as a system service without a
    /// config home).
    pub fn default_path() -> PathBuf {
        if let Some(dirs) = directories::ProjectDirs::from("dev", "systm-d", "alertu") {
            dirs.config_dir().join("config.toml")
        } else {
            PathBuf::from("/etc/alertu/config.toml")
        }
    }

    /// Load from `path`. A missing file yields defaults (not an error) so the
    /// daemon can start on a fresh system.
    pub fn load(path: &Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading config {}", path.display())),
        }
    }

    /// Serialize and atomically write to `path`, creating parent dirs.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializing config")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Point this config at a remote learned by pressing its button.
    ///
    /// Matches the remote by *name* rather than by node path: a Bluetooth remote
    /// drops off the bus when it sleeps and can return as a different
    /// `/dev/input/eventN`, which would silently unpair a path-pinned config. The
    /// path is used only when the kernel reports no usable name — mid-reconnect
    /// it can briefly be `?`.
    ///
    /// `toggle_keys` becomes exactly the key that was pressed. Keeping any
    /// previous entry would defeat the point: a key the remote cannot emit never
    /// matches, and that dead end is what guided pairing exists to remove.
    pub fn apply_learned_remote(&mut self, path: &Path, name: &str, key: &str) {
        let hint = name.trim();
        if hint.is_empty() || hint == "?" || hint == "<unnamed>" {
            self.remote_device = path.to_string_lossy().into_owned();
        } else {
            self.remote_device = AUTO.to_string();
            self.remote_name_hint = hint.to_string();
        }
        self.toggle_keys = vec![key.to_string()];
    }

    /// Basic sanity validation of user-provided values.
    pub fn validate(&self) -> Result<()> {
        if self.toggle_keys.is_empty() {
            anyhow::bail!("`toggle_keys` must list at least one evdev key name");
        }
        if self.watch_devices.is_empty() {
            anyhow::bail!("`watch_devices` must be `[\"auto\"]` or a list of device paths");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrips_through_toml() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn defaults_are_auto() {
        let cfg = Config::default();
        assert!(cfg.remote_is_auto());
        assert!(cfg.watch_is_auto());
        assert!(cfg.session_is_auto());
        cfg.validate().unwrap();
    }

    #[test]
    fn explicit_watch_is_not_auto() {
        let cfg = Config {
            watch_devices: vec!["/dev/input/event2".into()],
            ..Default::default()
        };
        assert!(!cfg.watch_is_auto());
    }

    #[test]
    fn a_named_learned_remote_is_matched_by_name() {
        let mut cfg = Config::default();
        cfg.apply_learned_remote(
            Path::new("/dev/input/event16"),
            "AB Shutter 6",
            "KEY_VOLUMEUP",
        );
        assert_eq!(cfg.remote_device, AUTO);
        assert_eq!(cfg.remote_name_hint, "AB Shutter 6");
        assert_eq!(cfg.toggle_keys, vec!["KEY_VOLUMEUP".to_string()]);
    }

    #[test]
    fn an_unnamed_learned_remote_falls_back_to_its_path() {
        let mut cfg = Config::default();
        cfg.apply_learned_remote(Path::new("/dev/input/event16"), "?", "KEY_SELECT");
        assert_eq!(cfg.remote_device, "/dev/input/event16");
        assert_eq!(cfg.toggle_keys, vec!["KEY_SELECT".to_string()]);
    }

    #[test]
    fn learning_replaces_previously_configured_keys() {
        let mut cfg = Config {
            toggle_keys: vec!["KEY_ENTER".to_string(), "KEY_A".to_string()],
            ..Default::default()
        };
        cfg.apply_learned_remote(Path::new("/dev/input/event1"), "Remote", "KEY_UP");
        assert_eq!(cfg.toggle_keys, vec!["KEY_UP".to_string()]);
    }

    #[test]
    fn validate_rejects_empty_toggle_keys() {
        let cfg = Config {
            toggle_keys: vec![],
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn unknown_field_is_rejected() {
        assert!(toml::from_str::<Config>("definitely_not_a_field = true\n").is_err());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let cfg = Config::load(std::path::Path::new(
            "/nonexistent/alertu/does-not-exist.toml",
        ))
        .unwrap();
        assert_eq!(cfg, Config::default());
    }
}
