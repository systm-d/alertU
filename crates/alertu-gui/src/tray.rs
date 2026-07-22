//! The StatusNotifierItem tray and its menu.
//!
//! The menu doubles as the "settings window": it reflects the current state and
//! lets the user pick the remote, choose watched devices, and nudge the tunable
//! delays. Menu callbacks never block — they queue a [`Request`] on an unbounded
//! channel, drained by the session loop in `main.rs`, and optimistically update
//! the local config so the menu redraws immediately.
//!
//! While the daemon is unreachable, queued requests are dropped rather than
//! replayed, so the action items are disabled and the tooltip says so.

use alertu_common::config::{AUTO, Config};
use alertu_common::protocol::{InputDeviceInfo, Request};
use alertu_common::state::GuardState;
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{Category, Icon, MenuItem, Status, ToolTip, Tray};
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;

/// The tray model. ksni calls the trait methods on its own task; we mutate it
/// only through `Handle::update`.
pub struct AlertuTray {
    pub state: GuardState,
    pub devices: Vec<InputDeviceInfo>,
    pub cfg: Option<Config>,
    pub req_tx: UnboundedSender<Request>,
    /// Daemon socket path, forwarded to the settings window when launched.
    pub socket: PathBuf,
    /// Whether the daemon connection is currently up. Actions that would send a
    /// request are disabled while it is down, since those requests are dropped.
    pub connected: bool,
}

impl AlertuTray {
    pub fn new(req_tx: UnboundedSender<Request>, socket: PathBuf) -> Self {
        AlertuTray {
            state: GuardState::Idle,
            devices: Vec::new(),
            cfg: None,
            req_tx,
            socket,
            connected: false,
        }
    }

    /// Queue a request, ignoring send errors (receiver gone → app closing).
    fn send(&self, req: Request) {
        let _ = self.req_tx.send(req);
    }

    /// Launch the standalone settings window (`alertu-settings`), pointed at the
    /// same daemon socket. Found on `PATH`; failure is logged, not fatal.
    fn launch_settings(&self) {
        match std::process::Command::new("alertu-settings")
            .arg("--socket")
            .arg(&self.socket)
            .spawn()
        {
            Ok(_) => {}
            Err(e) => warn!(error = %e, "could not launch alertu-settings (is it on PATH?)"),
        }
    }

    /// The path currently configured as the remote (empty when "auto").
    fn remote_path(&self) -> Option<PathBuf> {
        match &self.cfg {
            Some(cfg) if !cfg.remote_is_auto() => Some(PathBuf::from(&cfg.remote_device)),
            _ => None,
        }
    }

    /// The effective set of watched device paths, mirroring the daemon's
    /// resolution so toggling from "auto" produces a sensible explicit list.
    fn effective_watch(&self) -> Vec<PathBuf> {
        let Some(cfg) = &self.cfg else {
            return Vec::new();
        };
        let remote = self.remote_path();
        if cfg.watch_is_auto() {
            self.devices
                .iter()
                .filter(|d| !d.is_pointer && Some(&d.path) != remote.as_ref())
                .map(|d| d.path.clone())
                .collect()
        } else {
            cfg.watch_devices.iter().map(PathBuf::from).collect()
        }
    }
}

impl Tray for AlertuTray {
    fn id(&self) -> String {
        "dev.systm-d.alertu".into()
    }

    fn title(&self) -> String {
        "AlertU".into()
    }

    fn category(&self) -> Category {
        Category::SystemServices
    }

    fn status(&self) -> Status {
        match self.state {
            GuardState::Triggered | GuardState::Alarm => Status::NeedsAttention,
            _ => Status::Active,
        }
    }

    fn icon_name(&self) -> String {
        // Freedesktop-standard names; the theme falls back gracefully.
        match self.state {
            GuardState::Idle => "security-low",
            GuardState::Armed => "security-high",
            GuardState::Triggered => "dialog-warning",
            GuardState::Alarm => "dialog-error",
        }
        .into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        Vec::new()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "AlertU".into(),
            description: if self.connected {
                format!("State: {}", self.state)
            } else {
                "Daemon offline — reconnecting…".to_string()
            },
            icon_name: self.icon_name(),
            icon_pixmap: Vec::new(),
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = Vec::new();

        // State header (non-interactive).
        items.push(
            StandardItem {
                label: if self.connected {
                    format!("AlertU — {}", self.state)
                } else {
                    "AlertU — daemon offline".to_string()
                },
                enabled: false,
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);

        // Arm / Disarm.
        let toggle_label = match self.state {
            GuardState::Idle => "Arm",
            _ => "Disarm",
        };
        items.push(
            StandardItem {
                label: toggle_label.into(),
                enabled: self.connected,
                activate: Box::new(|tray: &mut AlertuTray| tray.send(Request::Toggle)),
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);
        items.push(self.remote_submenu());
        items.push(self.watch_submenu());
        items.push(self.settings_submenu());
        items.push(MenuItem::Separator);

        items.push(
            StandardItem {
                label: "Open settings…".into(),
                activate: Box::new(|tray: &mut AlertuTray| tray.launch_settings()),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Refresh devices".into(),
                enabled: self.connected,
                activate: Box::new(|tray: &mut AlertuTray| {
                    tray.send(Request::ListDevices);
                    tray.send(Request::GetConfig);
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Quit AlertU tray".into(),
                activate: Box::new(|_tray: &mut AlertuTray| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

impl AlertuTray {
    /// Submenu to pick the remote device (radio-style, marked with a bullet).
    fn remote_submenu(&self) -> MenuItem<Self> {
        let mut sub: Vec<MenuItem<Self>> = Vec::new();
        let current = self.remote_path();
        let auto = self.cfg.as_ref().is_some_and(|c| c.remote_is_auto());

        sub.push(
            StandardItem {
                label: format!("{}Auto (by name hint)", mark(auto)),
                activate: Box::new(|tray: &mut AlertuTray| {
                    if let Some(cfg) = tray.cfg.as_mut() {
                        cfg.remote_device = AUTO.into();
                        let snapshot = cfg.clone();
                        tray.send(Request::SetConfig(Box::new(snapshot)));
                    }
                }),
                ..Default::default()
            }
            .into(),
        );

        for dev in &self.devices {
            let selected = current.as_ref() == Some(&dev.path);
            let path = dev.path.clone();
            let label = format!(
                "{}{}{}",
                mark(selected),
                dev.name,
                if dev.is_pointer { "  [pointer]" } else { "" }
            );
            sub.push(
                StandardItem {
                    label,
                    activate: Box::new(move |tray: &mut AlertuTray| {
                        if let Some(cfg) = tray.cfg.as_mut() {
                            cfg.remote_device = path.to_string_lossy().into_owned();
                            let snapshot = cfg.clone();
                            tray.send(Request::SetConfig(Box::new(snapshot)));
                        }
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        if self.devices.is_empty() {
            sub.push(info_item("(no devices — click Refresh)"));
        }

        SubMenu {
            label: "Remote device".into(),
            submenu: sub,
            ..Default::default()
        }
        .into()
    }

    /// Submenu to pick watched devices (checkmarks). Toggling any device
    /// switches the config to an explicit list.
    fn watch_submenu(&self) -> MenuItem<Self> {
        let mut sub: Vec<MenuItem<Self>> = Vec::new();
        let auto = self.cfg.as_ref().is_some_and(|c| c.watch_is_auto());
        let watched = self.effective_watch();
        let remote = self.remote_path();

        sub.push(info_item(if auto {
            "Mode: auto (all except remote & mouse)"
        } else {
            "Mode: explicit selection"
        }));

        for dev in &self.devices {
            if Some(&dev.path) == remote.as_ref() {
                continue; // the remote is never watched
            }
            let checked = watched.contains(&dev.path);
            let path = dev.path.clone();
            sub.push(
                CheckmarkItem {
                    label: format!(
                        "{}{}",
                        dev.name,
                        if dev.is_pointer { "  [pointer]" } else { "" }
                    ),
                    checked,
                    activate: Box::new(move |tray: &mut AlertuTray| {
                        let mut set = tray.effective_watch();
                        if let Some(pos) = set.iter().position(|p| p == &path) {
                            set.remove(pos);
                        } else {
                            set.push(path.clone());
                        }
                        if let Some(cfg) = tray.cfg.as_mut() {
                            cfg.watch_devices = set
                                .iter()
                                .map(|p| p.to_string_lossy().into_owned())
                                .collect();
                            if cfg.watch_devices.is_empty() {
                                cfg.watch_devices = vec![AUTO.into()];
                            }
                            let snapshot = cfg.clone();
                            tray.send(Request::SetConfig(Box::new(snapshot)));
                        }
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        if self.devices.is_empty() {
            sub.push(info_item("(no devices — click Refresh)"));
        }

        SubMenu {
            label: "Watch devices".into(),
            submenu: sub,
            ..Default::default()
        }
        .into()
    }

    /// Submenu for the tunable delays plus read-only paths.
    fn settings_submenu(&self) -> MenuItem<Self> {
        let mut sub: Vec<MenuItem<Self>> = Vec::new();
        let cfg = self.cfg.clone();

        match &cfg {
            Some(cfg) => {
                sub.push(info_item(&format!(
                    "Alarm delay: {} s",
                    cfg.alarm_delay_secs
                )));
                sub.push(delta_item("  + increase alarm delay", |c| {
                    c.alarm_delay_secs = c.alarm_delay_secs.saturating_add(1);
                }));
                sub.push(delta_item("  − decrease alarm delay", |c| {
                    c.alarm_delay_secs = c.alarm_delay_secs.saturating_sub(1).max(1);
                }));

                sub.push(info_item(&format!(
                    "Grace period: {} s",
                    cfg.grace_period_secs
                )));
                sub.push(delta_item("  + increase grace period", |c| {
                    c.grace_period_secs = c.grace_period_secs.saturating_add(1);
                }));
                sub.push(delta_item("  − decrease grace period", |c| {
                    c.grace_period_secs = c.grace_period_secs.saturating_sub(1);
                }));

                sub.push(MenuItem::Separator);
                sub.push(info_item(&format!("Camera: {}", cfg.camera_device)));
                sub.push(info_item(&format!(
                    "Snapshots: {}",
                    cfg.snapshot_dir.display()
                )));
                sub.push(info_item(&format!("Siren: {}", cfg.siren_sound.display())));
                sub.push(info_item(&format!(
                    "Toggle keys: {}",
                    cfg.toggle_keys.join(", ")
                )));
            }
            None => sub.push(info_item("(config not loaded)")),
        }

        SubMenu {
            label: "Settings".into(),
            submenu: sub,
            ..Default::default()
        }
        .into()
    }
}

/// A disabled, informational menu row.
fn info_item(label: &str) -> MenuItem<AlertuTray> {
    StandardItem {
        label: label.to_string(),
        enabled: false,
        ..Default::default()
    }
    .into()
}

/// A menu row that mutates the config and pushes it to the daemon.
fn delta_item(label: &str, apply: fn(&mut Config)) -> MenuItem<AlertuTray> {
    StandardItem {
        label: label.to_string(),
        activate: Box::new(move |tray: &mut AlertuTray| {
            if let Some(cfg) = tray.cfg.as_mut() {
                apply(cfg);
                let snapshot = cfg.clone();
                tray.send(Request::SetConfig(Box::new(snapshot)));
            }
        }),
        ..Default::default()
    }
    .into()
}

/// Bullet prefix marking the currently-selected radio option.
fn mark(selected: bool) -> &'static str {
    if selected { "● " } else { "  " }
}
