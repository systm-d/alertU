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
    /// `remote_device = "auto"`. Defaults to the AB Shutter 3 reference device.
    pub remote_name_hint: String,

    /// evdev key name that arms/disarms, e.g. `KEY_VOLUMEUP` or `KEY_ENTER`.
    /// Any of the listed keys coming from the remote acts as the toggle.
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
            remote_name_hint: "AB Shutter".to_string(),
            toggle_keys: vec!["KEY_VOLUMEUP".to_string(), "KEY_ENTER".to_string()],
            watch_devices: vec![AUTO.to_string()],
            grace_period_secs: 15,
            alarm_delay_secs: 10,
            beep_sound: PathBuf::from("/usr/share/sounds/alertu/beep.wav"),
            warning_sound: PathBuf::from("/usr/share/sounds/alertu/warning.wav"),
            siren_sound: PathBuf::from("/usr/share/sounds/alertu/siren.wav"),
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
