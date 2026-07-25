//! The StatusNotifierItem tray and its menu.
//!
//! The menu is deliberately four entries: state, arm/disarm, pair, settings, quit.
//! It used to double as the settings window — device pickers, watch-list
//! checkboxes and delay nudges in nested submenus — which put a dozen choices in
//! front of a tray click. Those all live in `alertu-settings` now, under
//! "Advanced settings"; a tray menu is for the things you do while walking away
//! from the desk.
//!
//! Menu callbacks never block — they queue a [`Request`] on an unbounded channel,
//! drained by the session loop in `main.rs`, and optimistically update the local
//! config so the menu redraws immediately.
//!
//! While the daemon is unreachable, queued requests are dropped rather than
//! replayed, so the action items are disabled and the tooltip says so.

use alertu_common::config::Config;
use alertu_common::protocol::{InputDeviceInfo, Request};
use alertu_common::state::GuardState;
use ksni::menu::StandardItem;
use ksni::{Category, Icon, MenuItem, Status, ToolTip, Tray};
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;

/// How long the daemon listens for a button press during pairing.
///
/// Long enough to pick the remote up off the desk, short enough that a dialog
/// left open does not keep readers on every input device all afternoon.
const PAIRING_WINDOW_SECS: u64 = 30;

/// Where a pairing attempt stands, so the menu can say what to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pairing {
    /// Nothing attempted yet this session.
    Idle,
    /// Waiting for the user to press a button.
    Waiting,
    /// A key was learned and written to the daemon.
    Done { name: String, key: String },
    /// The window closed with no press.
    NothingPressed,
    /// The daemon refused (armed, or nothing to listen on).
    Refused,
}

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
    /// State of the guided pairing flow.
    pub pairing: Pairing,
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
            pairing: Pairing::Idle,
        }
    }

    /// Ask the daemon to watch for a button press.
    fn start_pairing(&mut self) {
        self.pairing = Pairing::Waiting;
        self.send(Request::LearnRemote {
            timeout_secs: PAIRING_WINDOW_SECS,
        });
    }

    /// Record the outcome of a pairing attempt and, on success, save it.
    ///
    /// `None` means the window closed with nothing pressed.
    pub fn finish_pairing(&mut self, learned: Option<(PathBuf, String, String)>) {
        let Some((path, name, key)) = learned else {
            self.pairing = Pairing::NothingPressed;
            return;
        };
        match self.cfg.as_mut() {
            Some(cfg) => {
                cfg.apply_learned_remote(&path, &name, &key);
                let snapshot = cfg.clone();
                self.send(Request::SetConfig(Box::new(snapshot)));
                self.pairing = Pairing::Done { name, key };
            }
            None => {
                // Nothing to fold the press into yet. Ask for the config so the
                // next attempt lands, rather than reporting a success that was
                // never written.
                warn!("learned a remote before the config arrived; re-requesting it");
                self.send(Request::GetConfig);
                self.pairing = Pairing::Refused;
            }
        }
    }

    /// A pairing attempt the daemon refused.
    ///
    /// Only meaningful while waiting: an unrelated error must not rewrite the
    /// result of a pairing that already finished.
    pub fn fail_pairing(&mut self) {
        if self.pairing == Pairing::Waiting {
            self.pairing = Pairing::Refused;
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

        items.push(self.pairing_item());
        items.push(MenuItem::Separator);

        items.push(
            StandardItem {
                label: "Settings…".into(),
                activate: Box::new(|tray: &mut AlertuTray| tray.launch_settings()),
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
    /// The one-click pairing entry.
    ///
    /// Top level rather than buried in the "Remote device" submenu, because this
    /// is the path that replaces the old one: read `list-devices`, work out
    /// which node is the remote, then guess the evdev name of the key it sends.
    /// Getting that last part wrong produced no error anywhere — just a remote
    /// that never armed — so pressing the button is the only reliable answer.
    fn pairing_item(&self) -> MenuItem<Self> {
        let label = match &self.pairing {
            Pairing::Waiting => return info_item("Press a button on your remote…"),
            Pairing::Idle => "Pair remote…".to_string(),
            Pairing::Done { name, key } => format!("Paired: {name} ({key}) — pair again"),
            Pairing::NothingPressed => "No button detected — pair again".to_string(),
            Pairing::Refused => "Pairing failed — disarm, then pair again".to_string(),
        };
        StandardItem {
            label,
            enabled: self.connected,
            activate: Box::new(|tray: &mut AlertuTray| tray.start_pairing()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    fn tray() -> AlertuTray {
        let (tx, _rx) = unbounded_channel();
        AlertuTray::new(tx, PathBuf::from("/run/alertu/alertu.sock"))
    }

    #[test]
    fn a_learned_press_is_saved_and_reported() {
        let mut t = tray();
        t.cfg = Some(Config::default());
        t.finish_pairing(Some((
            PathBuf::from("/dev/input/event16"),
            "AB Shutter 6".to_string(),
            "KEY_VOLUMEUP".to_string(),
        )));
        assert_eq!(
            t.pairing,
            Pairing::Done {
                name: "AB Shutter 6".to_string(),
                key: "KEY_VOLUMEUP".to_string(),
            }
        );
        let cfg = t.cfg.expect("config kept");
        assert_eq!(cfg.remote_name_hint, "AB Shutter 6");
        assert_eq!(cfg.toggle_keys, vec!["KEY_VOLUMEUP".to_string()]);
    }

    #[test]
    fn a_press_arriving_before_the_config_is_not_reported_as_paired() {
        let mut t = tray();
        assert!(t.cfg.is_none());
        t.finish_pairing(Some((
            PathBuf::from("/dev/input/event16"),
            "Remote".to_string(),
            "KEY_UP".to_string(),
        )));
        // Nothing was written, so claiming success would be a lie.
        assert_eq!(t.pairing, Pairing::Refused);
    }

    #[test]
    fn an_empty_window_says_no_button_was_pressed() {
        let mut t = tray();
        t.cfg = Some(Config::default());
        t.finish_pairing(None);
        assert_eq!(t.pairing, Pairing::NothingPressed);
    }

    #[test]
    fn an_unrelated_error_does_not_rewrite_a_finished_pairing() {
        let mut t = tray();
        t.cfg = Some(Config::default());
        t.finish_pairing(None);
        t.fail_pairing();
        assert_eq!(t.pairing, Pairing::NothingPressed);
    }
}
